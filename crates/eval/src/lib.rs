//! Differential evaluator. A benchmark passes when its translation compiles as
//! safe Rust and, with overflow checks on, prints the same standard output and
//! exits with the same code as the C original for every input.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// Directory whose rust-toolchain.toml pins rustc and Clippy.
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const RUN_TIMEOUT: &str = "10";
const TRANSPILE_TIMEOUT: &str = "30";
const COMPILE_TIMEOUT: &str = "60";
/// Exit codes of coreutils `timeout` when the limit is reached, and when the
/// command ignored SIGTERM and had to be killed.
const TIMEOUT_EXITS: [i32; 2] = [124, 137];
/// Exit code of a Rust program that panicked.
const PANIC_EXIT: i32 = 101;
/// Set for sanitizer reports so they differ from the program's own exit codes.
const SANITIZER_EXIT: i32 = 99;
const DETAIL_LIMIT: usize = 2000;

/// What a program printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit: Option<i32>,
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

/// Reads the inputs of `c_file` from the JSON file next to it, for example
/// `g1_gcd.json` for `g1_gcd.c`. Without that file the program runs once
/// without arguments or input.
pub fn inputs(c_file: &Path) -> Result<Vec<Input>, String> {
    let path = c_file.with_extension("json");
    let text = match std::fs::read(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(vec![Input::default()]);
        }
        Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
    };
    let invalid = |why: &str| format!("{}: {why}", path.display());
    let json: serde_json::Value =
        serde_json::from_slice(&text).map_err(|err| invalid(&err.to_string()))?;
    let list = json["inputs"]
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

/// Result for one benchmark program.
#[derive(Debug, Clone)]
pub struct Record {
    pub name: String,
    pub group: String,
    pub stage: &'static str,
    pub detail: String,
    /// Compiler and Clippy warnings by lint name.
    pub warnings: BTreeMap<String, u32>,
    pub seconds: f64,
}

impl Record {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "group": self.group,
            "stage": self.stage,
            "detail": self.detail,
            "warnings": self.warnings,
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
    if rust.stdout != c.stdout || rust.exit != c.exit {
        return ("mismatch", mismatch(c, rust));
    }
    if unsafe_uses > 0 {
        return ("unsafe", format!("{unsafe_uses} unsafe uses"));
    }
    ("pass", String::new())
}

/// Builds the transpiler binary and returns its path.
pub fn build_transpiler() -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args([
            "build",
            "-q",
            "-p",
            "transpiler",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(REPO)
        .output()
        .map_err(|err| format!("cannot run cargo: {err}"))?;
    if !output.status.success() {
        return Err("building the transpiler failed".to_owned());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        // The library artifact has the same name but no executable.
        .filter(|msg| msg["reason"] == "compiler-artifact" && msg["target"]["name"] == "transpiler")
        .find_map(|msg| msg["executable"].as_str().map(PathBuf::from))
        .ok_or_else(|| "cargo reported no transpiler executable".to_owned())
}

/// Evaluates one C program, using `work` as its scratch directory.
pub fn evaluate(c_file: &Path, transpiler: &Path, work: &Path) -> Record {
    let start = Instant::now();
    let name = c_file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let group = name.split('_').next().unwrap_or_default().to_owned();
    let mut warnings = BTreeMap::new();
    let (stage, detail) = stages(c_file, transpiler, work, &mut warnings);
    Record {
        name,
        group,
        stage,
        detail,
        warnings,
        seconds: (start.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    }
}

/// Builds `c_file` with AddressSanitizer and UndefinedBehaviorSanitizer.
pub fn build_c(c_file: &Path, program: &Path) -> Run {
    run(
        limited(COMPILE_TIMEOUT)
            .arg("clang-18")
            .args([
                "-g",
                "-fsanitize=address,undefined",
                "-fno-sanitize-recover=all",
                "-o",
            ])
            .arg(program)
            .arg(c_file)
            .arg("-lm"),
        &[],
    )
}

/// Runs `program` on `input` within the run time limit.
pub fn run_program(program: &Path, input: &Input) -> Run {
    run(
        limited(RUN_TIMEOUT).arg(program).args(&input.args),
        &input.stdin,
    )
}

/// Why a C run is not a valid reference, if it is not.
pub fn c_problem(c: &Run) -> Option<String> {
    if c.exit.is_none() {
        // For example an argument containing a NUL byte; the Rust side would fail alike.
        Some(format!("cannot run:\n{}", tail(&c.stderr)))
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
    transpiler: &Path,
    work: &Path,
    warnings: &mut BTreeMap<String, u32>,
) -> (&'static str, String) {
    let inputs = match inputs(c_file) {
        Ok(inputs) => inputs,
        Err(err) => return ("c_invalid", err),
    };
    let c_program = work.join("c_program");
    let built = build_c(c_file, &c_program);
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

    let rust_file = work.join("translation.rs");
    let transpiled = run(
        limited(TRANSPILE_TIMEOUT)
            .arg(transpiler)
            .arg(c_file)
            .arg("-o")
            .arg(&rust_file),
        &[],
    );
    if transpiled.exit != Some(0) || !rust_file.exists() {
        let reason = if transpiled.timed_out() {
            "timed out\n"
        } else {
            ""
        };
        return (
            "transpile_fail",
            format!("{reason}{}", tail(&transpiled.stderr)),
        );
    }

    let rust_program = work.join("rust_program");
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
    // Behaviour on every input comes first; `unsafe` is reported only after that.
    for (input, c) in inputs.iter().zip(&c_runs) {
        let rust = run_program(&rust_program, input);
        let (stage, detail) = classify(c, &rust, 0);
        if stage != "pass" {
            return (stage, on_input(input, detail));
        }
    }
    let unsafe_uses = warnings.get("unsafe_code").copied().unwrap_or(0);
    let last = c_runs.last().expect("inputs are never empty");
    classify(last, last, unsafe_uses)
}

fn compile(rust_file: &Path, program: &Path) -> Run {
    run(
        limited(COMPILE_TIMEOUT)
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
            .current_dir(REPO),
        &[],
    )
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
/// seconds later if they ignore SIGTERM.
fn limited(seconds: &str) -> Command {
    let mut command = Command::new("timeout");
    command.args(["--kill-after=2", seconds]);
    command
}

fn run(command: &mut Command, stdin: &[u8]) -> Run {
    let sanitizers = format!("exitcode={SANITIZER_EXIT}:detect_leaks=0");
    let child = command
        .env("ASAN_OPTIONS", &sanitizers)
        .env("UBSAN_OPTIONS", format!("exitcode={SANITIZER_EXIT}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let output = child.and_then(|mut child| {
        let mut pipe = child.stdin.take().expect("stdin is piped");
        // Written from another thread: a program that prints before reading
        // all of its input would otherwise block on a full output pipe.
        std::thread::scope(|scope| {
            // A program may exit without reading its input; that is not an error.
            scope.spawn(move || pipe.write_all(stdin));
            child.wait_with_output()
        })
    });
    match output {
        Ok(output) => Run {
            stdout: output.stdout,
            stderr: output.stderr,
            // A process killed by a signal has no exit code; use the shell's
            // 128 + signal, as `timeout` itself may be killed after --kill-after.
            exit: output
                .status
                .code()
                .or_else(|| output.status.signal().map(|signal| 128 + signal)),
        },
        Err(err) => Run {
            stdout: Vec::new(),
            stderr: format!("cannot run {:?}: {err}", command.get_program()).into_bytes(),
            exit: None,
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
            limited("1").args(["sh", "-c", "trap '' TERM; exec sleep 30"]),
            &[],
        );
        assert!(ran.timed_out(), "{:?}", ran.exit);
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
