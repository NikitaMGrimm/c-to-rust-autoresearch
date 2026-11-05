//! Differential evaluator. A benchmark passes when its translation compiles as
//! safe Rust and, with overflow checks on, prints the same standard output and
//! exits with the same code as the C original for every input.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Directory whose rust-toolchain.toml pins rustc and Clippy.
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const RUN_TIMEOUT: &str = "10";
const TRANSPILE_TIMEOUT: &str = "30";
const COMPILE_TIMEOUT: &str = "60";
const BUILD_TIMEOUT: &str = "600";
/// Exit codes of coreutils `timeout` when the limit is reached, and when the
/// command ignored SIGTERM and had to be killed.
const TIMEOUT_EXITS: [i32; 2] = [124, 137];
/// Exit code of a Rust program that panicked.
const PANIC_EXIT: i32 = 101;
/// Set for sanitizer reports so they differ from the program's own exit codes.
const SANITIZER_EXIT: i32 = 99;
const DETAIL_LIMIT: usize = 2000;
/// Bytes kept of a program's standard output and error; more is a failure.
const OUTPUT_LIMIT: u64 = 8 << 20;
/// Address space of the transpiler and translated programs. The C reference
/// has none: AddressSanitizer reserves terabytes of address space.
const MEMORY_LIMIT: &str = "--as=4294967296";
/// Largest file any program may write.
const FILE_LIMIT: &str = "--fsize=67108864";
/// Compiler options a benchmark may request for its C reference.
pub const C_FLAGS: [&str; 1] = ["-fno-strict-aliasing"];

/// What a program printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit: Option<i32>,
    /// Whether the program wrote more than the output limit.
    pub overflowed: bool,
}

impl Run {
    pub fn timed_out(&self) -> bool {
        self.exit.is_some_and(|code| TIMEOUT_EXITS.contains(&code))
    }

    /// Whether AddressSanitizer or UndefinedBehaviorSanitizer reported an error.
    pub fn sanitizer_error(&self) -> bool {
        // A program may exit with SANITIZER_EXIT itself; a report also prints one of these.
        let stderr = String::from_utf8_lossy(&self.stderr);
        self.exit == Some(SANITIZER_EXIT)
            && (stderr.contains("ERROR: AddressSanitizer") || stderr.contains("runtime error:"))
    }
}

/// Command-line arguments and standard input for one run of a program.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    pub name: String,
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
}

/// How a benchmark program is built and run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub inputs: Vec<Input>,
    /// Extra options for the C reference, from [`C_FLAGS`].
    pub c_flags: Vec<String>,
}

/// Reads the spec of `c_file` from the JSON file next to it, for example
/// `g1_gcd.json` for `g1_gcd.c`. Without that file the program runs once
/// without arguments or input.
pub fn spec(c_file: &Path) -> Result<Spec, String> {
    let path = c_file.with_extension("json");
    let text = match std::fs::read(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Spec {
                inputs: vec![Input::default()],
                c_flags: Vec::new(),
            });
        }
        Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
    };
    let invalid = |why: &str| format!("{}: {why}", path.display());
    let json: serde_json::Value =
        serde_json::from_slice(&text).map_err(|err| invalid(&err.to_string()))?;
    let c_flags = match &json["c_flags"] {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(flags) => flags
            .iter()
            .map(|flag| flag.as_str().filter(|flag| C_FLAGS.contains(flag)))
            .map(|flag| flag.map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or_else(|| invalid(&format!("\"c_flags\" may only contain {C_FLAGS:?}")))?,
        _ => return Err(invalid("\"c_flags\" must be a list")),
    };
    let inputs = parse_inputs(&json["inputs"], invalid)?;
    Ok(Spec { inputs, c_flags })
}

/// Extension of the file next to a benchmark program that holds its fuzzed
/// inputs, in the same form as the spec's `inputs`.
pub const FUZZ_EXTENSION: &str = "fuzz.json";

/// Reads the fuzzed inputs of `c_file`; a program may have none.
pub fn fuzzed_inputs(c_file: &Path) -> Result<Vec<Input>, String> {
    let path = c_file.with_extension(FUZZ_EXTENSION);
    let text = match std::fs::read(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
    };
    let invalid = |why: &str| format!("{}: {why}", path.display());
    let json: serde_json::Value =
        serde_json::from_slice(&text).map_err(|err| invalid(&err.to_string()))?;
    parse_inputs(&json["inputs"], invalid)
}

fn parse_inputs(
    list: &serde_json::Value,
    invalid: impl Fn(&str) -> String,
) -> Result<Vec<Input>, String> {
    let list = list
        .as_array()
        .filter(|list| !list.is_empty())
        .ok_or_else(|| invalid("expected a non-empty \"inputs\" list"))?;
    list.iter()
        .map(|input| {
            let name = input["name"]
                .as_str()
                .ok_or_else(|| invalid("every input needs a \"name\""))?;
            let args = match &input["args"] {
                serde_json::Value::Null => Vec::new(),
                serde_json::Value::Array(args) => args
                    .iter()
                    .map(|arg| arg.as_str().map(str::to_owned))
                    .collect::<Option<_>>()
                    .ok_or_else(|| invalid("\"args\" must be strings"))?,
                _ => return Err(invalid("\"args\" must be a list")),
            };
            let stdin = match &input["stdin"] {
                serde_json::Value::Null => Vec::new(),
                serde_json::Value::String(stdin) => stdin.as_bytes().to_vec(),
                _ => return Err(invalid("\"stdin\" must be a string")),
            };
            Ok(Input {
                name: name.to_owned(),
                args,
                stdin,
            })
        })
        .collect()
}

/// Where translations come from.
#[derive(Debug, Clone)]
pub enum Translator {
    /// The transpiler executable, run on each C program.
    Transpiler(PathBuf),
    /// A directory with an existing translation `NAME.rs` for each `NAME.c`.
    Existing(PathBuf),
}

/// Result for one benchmark program.
#[derive(Debug, Clone)]
pub struct Record {
    pub name: String,
    pub group: String,
    pub stage: &'static str,
    pub detail: String,
    /// Compiler and Clippy warnings by lint name.
    pub warnings: BTreeMap<String, u32>,
    /// Idiomaticity counts of a translation that compiled.
    pub metrics: Option<BTreeMap<&'static str, u32>>,
    /// Stage and detail on the fuzzed inputs, for a translation that passed
    /// and a program that has them.
    pub fuzz: Option<(&'static str, String)>,
    pub seconds: f64,
}

impl Record {
    /// A pass that does not hold on the program's fuzzed inputs.
    pub fn fails_fuzz(&self) -> bool {
        self.stage == "pass"
            && self
                .fuzz
                .as_ref()
                .is_some_and(|(stage, _)| *stage != "pass")
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "group": self.group,
            "stage": self.stage,
            "detail": self.detail,
            "warnings": self.warnings,
            "metrics": self.metrics,
            "fuzz": self.fuzz.as_ref().map(|(stage, detail)| {
                serde_json::json!({"stage": stage, "detail": detail})
            }),
            "seconds": self.seconds,
        })
    }
}

/// Decides the stage of a translation that compiled, given both runs.
pub fn classify(c: &Run, rust: &Run, unsafe_uses: u32) -> (&'static str, String) {
    if rust.timed_out() {
        return ("timeout", String::new());
    }
    // A C program may exit with 101 too; only the panic message tells them apart.
    if rust.exit == Some(PANIC_EXIT)
        && String::from_utf8_lossy(&rust.stderr).contains("panicked at")
    {
        return ("panic", tail(&rust.stderr));
    }
    if rust.overflowed {
        return ("mismatch", format!("output exceeds {OUTPUT_LIMIT} bytes"));
    }
    if rust.stdout != c.stdout || rust.exit != c.exit {
        return ("mismatch", mismatch(c, rust));
    }
    if unsafe_uses > 0 {
        return ("unsafe", format!("{unsafe_uses} unsafe uses"));
    }
    ("pass", String::new())
}

const BUILD_ARGS: [&str; 5] = [
    "build",
    "-q",
    "-p",
    "transpiler",
    "--message-format=json-render-diagnostics",
];

/// Builds the transpiler binary and returns its path.
pub fn build_transpiler() -> Result<PathBuf, String> {
    cargo_executable(Command::new("cargo").args(BUILD_ARGS).current_dir(REPO))
}

/// Builds the transpiler of an untrusted `workspace` in the sandbox, so its
/// build scripts see neither the private benchmarks nor credentials.
/// Dependencies are fetched first, outside, since the sandbox has no network.
pub fn build_transpiler_sandboxed(workspace: &Path) -> Result<PathBuf, String> {
    let workspace = workspace
        .canonicalize()
        .map_err(|err| format!("cannot open {}: {err}", workspace.display()))?;
    let fetched = Command::new("cargo")
        .args(["fetch", "-q", "--locked"])
        .current_dir(&workspace)
        .status()
        .map_err(|err| format!("cannot run cargo: {err}"))?;
    if !fetched.success() {
        return Err("fetching the transpiler's dependencies failed".to_owned());
    }
    let toolchain = toolchain();
    let readable: Vec<&Path> = toolchain.iter().map(PathBuf::as_path).collect();
    let mut inner = Command::new("timeout");
    inner
        .args(["--kill-after=2", BUILD_TIMEOUT, "cargo"])
        .args(BUILD_ARGS)
        .args(["--offline", "--locked"])
        .env("CARGO_TARGET_DIR", workspace.join("target"))
        .current_dir(&workspace);
    cargo_executable(&mut sandboxed(&inner, &readable, &workspace))
}

/// What cargo and rustc need inside the sandbox. Not all of CARGO_HOME: it
/// holds the crates.io token.
fn toolchain() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let cargo_home = std::env::var_os("CARGO_HOME").map_or(home.join(".cargo"), PathBuf::from);
    let rustup_home = std::env::var_os("RUSTUP_HOME").map_or(home.join(".rustup"), PathBuf::from);
    ["bin", "registry", "git"]
        .iter()
        .map(|dir| cargo_home.join(dir))
        .chain([rustup_home])
        .filter(|path| path.exists())
        .collect()
}

fn cargo_executable(command: &mut Command) -> Result<PathBuf, String> {
    let output = command
        .output()
        .map_err(|err| format!("cannot run cargo: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "building the transpiler failed\n{}",
            tail(&output.stderr)
        ));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        // The library artifact has the same name but no executable.
        .filter(|msg| msg["reason"] == "compiler-artifact" && msg["target"]["name"] == "transpiler")
        .find_map(|msg| msg["executable"].as_str().map(PathBuf::from))
        .ok_or_else(|| "cargo reported no transpiler executable".to_owned())
}

/// Tool versions and the repository commit, so results can be compared.
pub fn versions() -> serde_json::Value {
    let first_line = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .current_dir(REPO)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| {
                let text = String::from_utf8_lossy(&output.stdout);
                text.lines().next().unwrap_or_default().trim().to_owned()
            })
    };
    let dirty = first_line("git", &["status", "--porcelain", "--untracked-files=no"])
        .map(|line| !line.is_empty());
    serde_json::json!({
        "commit": first_line("git", &["rev-parse", "HEAD"]),
        "uncommitted_changes": dirty,
        "clang": first_line("clang-18", &["--version"]),
        "rustc": first_line("rustc", &["--version"]),
    })
}

/// The `.c` programs in `dir`, sorted.
pub fn c_programs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).and_then(|entries| {
        entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
    });
    let mut programs: Vec<PathBuf> = entries
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .collect();
    programs.sort();
    Ok(programs)
}

/// Evaluates `programs` on `jobs` threads; records are sorted by name.
pub fn evaluate_all(
    programs: &[PathBuf],
    translator: &Translator,
    jobs: usize,
) -> Result<Vec<Record>, String> {
    // Private and new: a planted directory or symlink at this path must not be used.
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let scratch = std::env::temp_dir().join(format!(
        "c2r-eval-{}-{}",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&scratch)
        .map_err(|err| format!("cannot create {}: {err}", scratch.display()))?;
    for program in programs {
        if let Err(err) = std::fs::create_dir(scratch.join(program.file_stem().unwrap_or_default()))
        {
            let _ = std::fs::remove_dir_all(&scratch);
            return Err(format!("cannot create scratch directory: {err}"));
        }
    }
    let next = AtomicUsize::new(0);
    let records = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(programs.len()) {
            scope.spawn(|| {
                while let Some(program) = programs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let work = scratch.join(program.file_stem().unwrap_or_default());
                    let record = evaluate(program, translator, &work);
                    records.lock().unwrap().push(record);
                }
            });
        }
    });
    let _ = std::fs::remove_dir_all(&scratch);
    let mut records = records.into_inner().unwrap();
    records.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(records)
}

/// Evaluates one C program, using `work` as its scratch directory.
pub fn evaluate(c_file: &Path, translator: &Translator, work: &Path) -> Record {
    let start = Instant::now();
    let name = c_file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let group = name.split('_').next().unwrap_or_default().to_owned();
    let mut warnings = BTreeMap::new();
    let (mut metrics, mut fuzz) = (None, None);
    let (stage, detail) = stages(
        c_file,
        translator,
        work,
        &mut warnings,
        &mut metrics,
        &mut fuzz,
    );
    Record {
        name,
        group,
        stage,
        detail,
        warnings,
        metrics,
        fuzz,
        seconds: (start.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    }
}

/// Builds `c_file` with AddressSanitizer and UndefinedBehaviorSanitizer.
pub fn build_c(c_file: &Path, program: &Path, c_flags: &[String]) -> Run {
    run(
        limited(COMPILE_TIMEOUT, false)
            .arg("clang-18")
            .args([
                "-g",
                "-fsanitize=address,undefined",
                "-fno-sanitize-recover=all",
            ])
            .args(c_flags)
            .arg("-o")
            .arg(program)
            .arg(c_file)
            .arg("-lm"),
        &[],
    )
}

/// Builds of a C program that must agree before its runs serve as reference.
/// Optimised builds without sanitizers often expose undefined behaviour that
/// the sanitizers miss, by printing something else.
pub struct References {
    sanitized: PathBuf,
    optimised: Vec<PathBuf>,
}

impl References {
    /// Builds `c_file` sanitized, and with `clang-18 -O2` and `gcc -O2`, in
    /// subdirectories of `work`.
    pub fn build(c_file: &Path, c_flags: &[String], work: &Path) -> Result<Self, String> {
        let mut programs = Vec::new();
        for (name, compiler) in [
            ("sanitized", None),
            ("clang_O2", Some("clang-18")),
            ("gcc_O2", Some("gcc")),
        ] {
            let dir = work.join(name);
            std::fs::create_dir_all(&dir)
                .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
            let program = dir.join("program");
            let built = match compiler {
                None => build_c(c_file, &program, c_flags),
                Some(compiler) => run(
                    limited(COMPILE_TIMEOUT, false)
                        .arg(compiler)
                        .args(["-O2", "-w"])
                        .args(c_flags)
                        .arg("-o")
                        .arg(&program)
                        .arg(c_file)
                        .arg("-lm"),
                    &[],
                ),
            };
            if built.exit != Some(0) {
                return Err(format!("c_build: {name}"));
            }
            programs.push(program);
        }
        let sanitized = programs.remove(0);
        Ok(Self {
            sanitized,
            optimised: programs,
        })
    }

    /// The sanitized run of `input`, if it is valid, the same twice, and the
    /// same as the optimised builds' runs; otherwise why not.
    pub fn check(&self, input: &Input) -> Result<Run, &'static str> {
        let first = run_program(&self.sanitized, input);
        if first.timed_out() {
            return Err("c_timeout");
        }
        if first.sanitizer_error() {
            return Err("c_sanitizer_error");
        }
        if c_problem(&first).is_some() {
            return Err("c_unusable");
        }
        if run_program(&self.sanitized, input) != first {
            return Err("c_nondeterministic");
        }
        for program in &self.optimised {
            let other = run_program(program, input);
            if (&other.stdout, other.exit) != (&first.stdout, first.exit) {
                return Err("c_builds_disagree");
            }
        }
        Ok(first)
    }
}

/// SplitMix64 finaliser: spreads a seed into an evenly distributed value.
pub fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Deterministic random numbers from a seed, so generated benchmarks can be
/// regenerated exactly.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        mix(self.0)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// FNV-1a hash, to derive seeds from names.
pub fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

/// Runs the C program `program` on `input` within the run time limit.
pub fn run_program(program: &Path, input: &Input) -> Run {
    run_in_place(program, input, false)
}

/// Runs `program` from its own directory as `./NAME`, so C and Rust programs
/// with the same file name see the same `argv[0]` and write files apart.
fn run_in_place(program: &Path, input: &Input, memory_limit: bool) -> Run {
    let name = Path::new(".").join(program.file_name().unwrap_or_default());
    let dir = program.parent().unwrap_or(Path::new("."));
    let mut command = limited(RUN_TIMEOUT, memory_limit);
    command.arg(name).args(&input.args).current_dir(dir);
    // Only translations get the memory limit, and only they are agent-written.
    if memory_limit {
        command = sandboxed(&command, &[], dir);
    }
    run(&mut command, &input.stdin)
}

/// Wraps agent-written code: no network, a read-only file system except
/// `writable`, and a hidden home directory, where the private benchmarks and
/// credentials live, except `readable`. Paths must be absolute.
fn sandboxed(inner: &Command, readable: &[&Path], writable: &Path) -> Command {
    let mut command = Command::new("bwrap");
    command.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]);
    command.args(["--tmpfs", "/tmp"]);
    if let Some(home) = std::env::var_os("HOME") {
        command.arg("--tmpfs").arg(home);
    }
    for path in readable {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    command.arg("--bind").arg(writable).arg(writable);
    if let Some(dir) = inner.get_current_dir() {
        command.arg("--chdir").arg(dir);
    }
    command.args(["--unshare-all", "--die-with-parent", "--new-session", "--"]);
    command.arg(inner.get_program()).args(inner.get_args());
    for (key, value) in inner.get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command
}

/// Why a C run is not a valid reference, if it is not.
pub fn c_problem(c: &Run) -> Option<String> {
    if c.exit.is_none() {
        // For example an argument containing a NUL byte; the Rust side would fail alike.
        Some(format!("cannot run:\n{}", tail(&c.stderr)))
    } else if c.overflowed {
        Some(format!("C output exceeds {OUTPUT_LIMIT} bytes"))
    } else if c.timed_out() {
        Some("C program timed out".to_owned())
    } else if c.sanitizer_error() {
        Some(format!("sanitizer error:\n{}", tail(&c.stderr)))
    } else {
        None
    }
}

/// Names the input in a failure detail when the program has named inputs.
fn on_input(input: &Input, detail: String) -> String {
    if input.name.is_empty() {
        detail
    } else {
        format!("input {}: {detail}", input.name)
    }
}

fn stages(
    c_file: &Path,
    translator: &Translator,
    work: &Path,
    warnings: &mut BTreeMap<String, u32>,
    metrics: &mut Option<BTreeMap<&'static str, u32>>,
    fuzz: &mut Option<(&'static str, String)>,
) -> (&'static str, String) {
    let Spec { inputs, c_flags } = match spec(c_file) {
        Ok(spec) => spec,
        Err(err) => return ("c_invalid", err),
    };
    for dir in [work.join("c"), work.join("rust")] {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            return (
                "c_invalid",
                format!("cannot create {}: {err}", dir.display()),
            );
        }
    }
    // Everything agent-written stays in `rust_dir`, the sandbox's only
    // writable directory, which needs an absolute path.
    let rust_dir = match work.join("rust").canonicalize() {
        Ok(dir) => dir,
        Err(err) => {
            return (
                "c_invalid",
                format!("cannot open {}: {err}", work.display()),
            );
        }
    };
    // The same file name in both directories gives both the same argv[0].
    let (c_program, rust_program) = (work.join("c/program"), rust_dir.join("program"));
    let built = build_c(c_file, &c_program, &c_flags);
    if built.exit != Some(0) {
        return (
            "c_invalid",
            format!("C build failed:\n{}", tail(&built.stderr)),
        );
    }
    let mut c_runs = Vec::new();
    for input in &inputs {
        let c = run_program(&c_program, input);
        if let Some(problem) = c_problem(&c) {
            return ("c_invalid", on_input(input, problem));
        }
        c_runs.push(c);
    }

    let rust_file = rust_dir.join("translation.rs");
    if let Err(detail) = translate(c_file, translator, &rust_file) {
        return ("transpile_fail", detail);
    }
    let compiled = compile(&rust_file, &rust_program);
    let errors = diagnostics(&compiled.stderr, warnings);
    if compiled.exit != Some(0) {
        // Without JSON errors (spawn failure, timeout), show the raw output.
        let detail = if errors.is_empty() {
            tail(&compiled.stderr)
        } else {
            errors
        };
        return ("compile_fail", detail);
    }
    *metrics = std::fs::read_to_string(&rust_file)
        .ok()
        .and_then(|source| idiom_counts(&source));
    // Behaviour on every input comes first; `unsafe` is reported only after that.
    for (input, c) in inputs.iter().zip(&c_runs) {
        let rust = run_in_place(&rust_program, input, true);
        let (stage, detail) = classify(c, &rust, 0);
        if stage != "pass" {
            return (stage, on_input(input, detail));
        }
    }
    let unsafe_uses = warnings.get("unsafe_code").copied().unwrap_or(0);
    let last = c_runs.last().expect("inputs are never empty");
    let result = classify(last, last, unsafe_uses);
    if result.0 == "pass" {
        *fuzz = fuzz_stage(c_file, &c_program, &rust_program);
    }
    result
}

/// How a passing translation does on the fuzzed inputs, if there are any.
/// They are a separate result, so the pass rule stays as it was.
fn fuzz_stage(
    c_file: &Path,
    c_program: &Path,
    rust_program: &Path,
) -> Option<(&'static str, String)> {
    let inputs = match fuzzed_inputs(c_file) {
        Ok(inputs) if inputs.is_empty() => return None,
        Ok(inputs) => inputs,
        Err(err) => return Some(("c_invalid", err)),
    };
    for input in &inputs {
        let c = run_program(c_program, input);
        if let Some(problem) = c_problem(&c) {
            return Some(("c_invalid", on_input(input, problem)));
        }
        let rust = run_in_place(rust_program, input, true);
        let (stage, detail) = classify(&c, &rust, 0);
        if stage != "pass" {
            return Some((stage, on_input(input, detail)));
        }
    }
    Some(("pass", String::new()))
}

/// Writes the translation of `c_file` to `rust_file`, or says why it could not.
fn translate(c_file: &Path, translator: &Translator, rust_file: &Path) -> Result<(), String> {
    match translator {
        Translator::Transpiler(transpiler) => {
            let absolute = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
            let (transpiler, c_file) = (absolute(transpiler), absolute(c_file));
            let out_dir = absolute(rust_file.parent().unwrap_or(Path::new(".")));
            let mut command = limited(TRANSPILE_TIMEOUT, true);
            command
                .arg(&transpiler)
                .arg(&c_file)
                .arg("-o")
                .arg(out_dir.join(rust_file.file_name().unwrap_or_default()));
            let transpiled = run(
                &mut sandboxed(&command, &[&transpiler, &c_file], &out_dir),
                &[],
            );
            if transpiled.exit == Some(0) && rust_file.exists() {
                return Ok(());
            }
            let reason = if transpiled.timed_out() {
                "timed out\n"
            } else {
                ""
            };
            Err(format!("{reason}{}", tail(&transpiled.stderr)))
        }
        Translator::Existing(dir) => {
            let existing = dir
                .join(c_file.file_name().unwrap_or_default())
                .with_extension("rs");
            std::fs::copy(&existing, rust_file)
                .map(drop)
                .map_err(|err| format!("no translation {}: {err}", existing.display()))
        }
    }
}

/// Counts constructs that show how idiomatic a translation is. Counting the
/// syntax tree ignores comments and strings that mention them.
pub fn idiom_counts(source: &str) -> Option<BTreeMap<&'static str, u32>> {
    use syn::visit::Visit;

    struct Counter(BTreeMap<&'static str, u32>);
    impl Counter {
        fn add(&mut self, name: &'static str) {
            *self.0.entry(name).or_default() += 1;
        }
    }
    impl<'ast> Visit<'ast> for Counter {
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if call.method == "clone" && call.args.is_empty() {
                self.add("clones");
            }
            syn::visit::visit_expr_method_call(self, call);
        }
        fn visit_type_ptr(&mut self, ptr: &'ast syn::TypePtr) {
            self.add("raw_pointers");
            syn::visit::visit_type_ptr(self, ptr);
        }
        fn visit_expr_raw_addr(&mut self, expr: &'ast syn::ExprRawAddr) {
            self.add("raw_pointers");
            syn::visit::visit_expr_raw_addr(self, expr);
        }
        fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
            if wraps(&ty.path, "Rc", "RefCell") {
                self.add("rc_refcell");
            }
            syn::visit::visit_type_path(self, ty);
        }
        fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
            // Lint counts are only meaningful next to the lints switched off.
            if attr.path().is_ident("allow") || attr.path().is_ident("expect") {
                self.add("lint_allows");
            }
            syn::visit::visit_attribute(self, attr);
        }
    }

    fn wraps(path: &syn::Path, outer: &str, inner: &str) -> bool {
        let Some(last) = path.segments.last().filter(|last| last.ident == outer) else {
            return false;
        };
        let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
            return false;
        };
        args.args.iter().any(|arg| {
            matches!(arg, syn::GenericArgument::Type(syn::Type::Path(ty))
                if ty.path.segments.last().is_some_and(|segment| segment.ident == inner))
        })
    }

    let file = syn::parse_file(source).ok()?;
    let names = ["clones", "raw_pointers", "rc_refcell", "lint_allows"];
    let mut counter = Counter(names.into_iter().map(|name| (name, 0)).collect());
    counter.visit_file(&file);
    let lines = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    counter
        .0
        .insert("lines", u32::try_from(lines).unwrap_or(u32::MAX));
    Some(counter.0)
}

/// Compiles in the sandbox, since `include_str!` and the like read files.
fn compile(rust_file: &Path, program: &Path) -> Run {
    let repo = Path::new(REPO)
        .canonicalize()
        .unwrap_or_else(|_| REPO.into());
    let mut readable = toolchain();
    readable.push(repo.join("rust-toolchain.toml"));
    let readable: Vec<&Path> = readable.iter().map(PathBuf::as_path).collect();
    let mut command = limited(COMPILE_TIMEOUT, false);
    command
        .arg("clippy-driver")
        .args([
            "--edition",
            "2024",
            "-C",
            "overflow-checks=on",
            "-C",
            "debug-assertions=on",
        ])
        // force-warn: an `allow(unsafe_code)` in the translation cannot hide it.
        .args(["--force-warn", "unsafe-code", "--error-format=json", "-o"])
        .arg(program)
        .arg(rust_file)
        // Clippy looks for its configuration there; it is hidden, and there is none.
        .env_remove("CARGO_MANIFEST_DIR")
        .current_dir(&repo);
    let dir = program.parent().unwrap_or(Path::new("/"));
    run(&mut sandboxed(&command, &readable, dir), &[])
}

/// Counts warnings by lint name and returns the rendered errors.
fn diagnostics(stderr: &[u8], warnings: &mut BTreeMap<String, u32>) -> String {
    let mut errors = String::new();
    for msg in String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
    {
        match msg["level"].as_str() {
            Some("warning") => {
                if let Some(code) = msg["code"]["code"].as_str() {
                    *warnings.entry(code.to_owned()).or_default() += 1;
                }
            }
            Some("error") if errors.len() < DETAIL_LIMIT => {
                errors.push_str(msg["rendered"].as_str().unwrap_or_default());
            }
            _ => {}
        }
    }
    errors.truncate(errors.floor_char_boundary(DETAIL_LIMIT));
    errors
}

/// A command that runs its arguments for at most `seconds`, killing them two
/// seconds later if they ignore SIGTERM, with a file size limit and, if asked,
/// a memory limit.
fn limited(seconds: &str, memory_limit: bool) -> Command {
    let mut command = Command::new("prlimit");
    command.arg(FILE_LIMIT);
    if memory_limit {
        command.arg(MEMORY_LIMIT);
    }
    command.args(["--", "timeout", "--kill-after=2", seconds]);
    command
}

/// Reads up to the output limit, then discards the rest so the program never
/// blocks on a full pipe. Returns whether anything was discarded.
fn capped(pipe: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    let mut pipe = pipe;
    let mut kept = Vec::new();
    (&mut pipe).take(OUTPUT_LIMIT).read_to_end(&mut kept)?;
    let discarded = std::io::copy(&mut pipe, &mut std::io::sink())?;
    Ok((kept, discarded > 0))
}

fn run(command: &mut Command, stdin: &[u8]) -> Run {
    // The memory limit cannot apply under AddressSanitizer; it watches RSS instead.
    let sanitizers = format!("exitcode={SANITIZER_EXIT}:detect_leaks=0:hard_rss_limit_mb=4096");
    let child = command
        .env("ASAN_OPTIONS", &sanitizers)
        .env("UBSAN_OPTIONS", format!("exitcode={SANITIZER_EXIT}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let output = child.and_then(|mut child| {
        let mut pipe = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        // Each pipe has its own thread: a program that prints before reading
        // all of its input would otherwise block on a full pipe.
        std::thread::scope(|scope| {
            // A program may exit without reading its input; that is not an error.
            scope.spawn(move || pipe.write_all(stdin));
            let stdout = scope.spawn(move || capped(stdout));
            let stderr = scope.spawn(move || capped(stderr));
            let status = child.wait()?;
            let (stdout, out_overflowed) = stdout.join().expect("reader panicked")?;
            let (stderr, err_overflowed) = stderr.join().expect("reader panicked")?;
            Ok((status, stdout, stderr, out_overflowed || err_overflowed))
        })
    });
    match output {
        Ok((status, stdout, stderr, overflowed)) => Run {
            stdout,
            stderr,
            // A process killed by a signal has no exit code; use the shell's
            // 128 + signal, as `timeout` itself may be killed after --kill-after.
            exit: status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal)),
            overflowed,
        },
        Err(err) => Run {
            stdout: Vec::new(),
            stderr: format!("cannot run {:?}: {err}", command.get_program()).into_bytes(),
            exit: None,
            overflowed: false,
        },
    }
}

fn mismatch(c: &Run, rust: &Run) -> String {
    let expected = String::from_utf8_lossy(&c.stdout);
    let got = String::from_utf8_lossy(&rust.stdout);
    let mut detail = format!("exit: C {:?}, Rust {:?}\n", c.exit, rust.exit);
    if c.stdout == rust.stdout {
        detail.push_str("output identical");
        return detail;
    }
    // Lines keep their terminators so a missing final newline shows up.
    let lines = |text: &str| {
        text.split_inclusive('\n')
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let (expected, got) = (lines(&expected), lines(&got));
    let first_difference = expected.iter().zip(&got).position(|(a, b)| a != b);
    let line = first_difference.unwrap_or_else(|| expected.len().min(got.len()));
    detail.push_str(&format!(
        "first differing line {}:\n  C:    {:?}\n  Rust: {:?}",
        line + 1,
        expected.get(line).map_or("<end of output>", String::as_str),
        got.get(line).map_or("<end of output>", String::as_str),
    ));
    detail
}

fn tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let start = text.ceil_char_boundary(text.len().saturating_sub(DETAIL_LIMIT));
    text[start..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(stdout: &str, exit: i32) -> Run {
        Run {
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
            exit: Some(exit),
            overflowed: false,
        }
    }

    #[test]
    fn same_output_and_exit_passes() {
        assert_eq!(classify(&ran("1\n", 3), &ran("1\n", 3), 0).0, "pass");
    }

    #[test]
    fn different_exit_code_is_a_mismatch() {
        assert_eq!(classify(&ran("1\n", 0), &ran("1\n", 1), 0).0, "mismatch");
    }

    #[test]
    fn exit_only_mismatch_says_output_is_identical() {
        let (_, detail) = classify(&ran("1\n", 3), &ran("1\n", 0), 0);
        assert!(detail.ends_with("output identical"), "{detail}");
    }

    #[test]
    fn missing_line_is_a_mismatch() {
        let (stage, detail) = classify(&ran("1\n2\n", 0), &ran("1\n", 0), 0);
        assert_eq!(stage, "mismatch");
        assert!(detail.contains("line 2"), "{detail}");
    }

    #[test]
    fn correct_unsafe_translation_fails() {
        assert_eq!(classify(&ran("1\n", 0), &ran("1\n", 0), 2).0, "unsafe");
    }

    #[test]
    fn panic_and_timeout_are_reported() {
        let mut panicked = ran("", PANIC_EXIT);
        panicked.stderr = b"thread 'main' panicked at src/main.rs:1:1".to_vec();
        assert_eq!(classify(&ran("", 0), &panicked, 0).0, "panic");
        assert_eq!(classify(&ran("", 0), &ran("", PANIC_EXIT), 0).0, "mismatch");
        assert_eq!(classify(&ran("", 0), &ran("", 124), 0).0, "timeout");
    }

    #[test]
    fn missing_final_newline_is_shown() {
        let (_, detail) = classify(&ran("hi\n", 0), &ran("hi", 0), 0);
        assert!(detail.contains(r#""hi\n""#), "{detail}");
    }

    #[test]
    fn unsafe_code_is_counted_despite_allow() {
        let dir = std::env::temp_dir().join(format!("eval-unsafe-allow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("translation.rs");
        std::fs::write(
            &source,
            "#![allow(unsafe_code)]\nfn main() {\n    let x = 1;\n    let p = &raw const x;\n    println!(\"{}\", unsafe { *p });\n}\n",
        )
        .unwrap();

        let compiled = compile(&source, &dir.join("program"));
        let mut warnings = BTreeMap::new();
        let errors = diagnostics(&compiled.stderr, &mut warnings);
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(compiled.exit, Some(0), "{errors}");
        assert_eq!(warnings.get("unsafe_code"), Some(&1));
    }

    #[test]
    fn program_ignoring_sigterm_times_out() {
        let ran = run(
            limited("1", false).args(["sh", "-c", "trap '' TERM; exec sleep 30"]),
            &[],
        );
        assert!(ran.timed_out(), "{:?}", ran.exit);
    }

    #[test]
    fn idiom_counts_come_from_code_not_comments() {
        let source = r#"#![allow(dead_code)]
use std::cell::RefCell;
use std::rc::Rc;
// x.clone() in a comment does not count
fn main() {
    let shared: Rc<RefCell<i32>> = Rc::new(RefCell::new(1));
    let other = shared.clone();
    let x = 1;
    let p: *const i32 = &raw const x;
    println!("{:?} {:?} clone()", other, p);
}
"#;
        let counts = idiom_counts(source).unwrap();
        assert_eq!(counts["clones"], 1);
        assert_eq!(counts["raw_pointers"], 2);
        assert_eq!(counts["rc_refcell"], 1);
        assert_eq!(counts["lint_allows"], 1);
        assert_eq!(counts["lines"], 11);
        assert_eq!(idiom_counts("fn main( {"), None);
    }

    #[test]
    fn warnings_are_counted_by_lint() {
        let stderr = br#"{"level":"warning","code":{"code":"unsafe_code"},"rendered":"w"}
{"level":"warning","code":{"code":"unsafe_code"},"rendered":"w"}
{"level":"warning","code":null,"rendered":"2 warnings emitted"}
{"level":"error","code":null,"rendered":"error: boom\n"}"#;
        let mut warnings = BTreeMap::new();
        let errors = diagnostics(stderr, &mut warnings);
        assert_eq!(warnings.get("unsafe_code"), Some(&2));
        assert_eq!(warnings.len(), 1);
        assert_eq!(errors, "error: boom\n");
    }
}
