//! Evaluating translations: the stages a translation passes through and the
//! record of where it stopped.

use std::collections::BTreeMap;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::REPO;
use crate::idioms::idiom_counts;
use crate::reference::{build_c, c_problem};
use crate::run::{OUTPUT_LIMIT, Run, run_in_place, run_program, tail};
use crate::spec::{Input, Spec, fuzzed_inputs, spec};
use crate::translate::{Translator, compile, diagnostics, translate};

/// Exit code of a Rust program that panicked.
const PANIC_EXIT: i32 = 101;

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

/// Passes whose program has fuzzed inputs, and how many of them hold on them.
pub fn fuzz_counts(records: &[Record]) -> (usize, usize) {
    let fuzzed: Vec<&Record> = records
        .iter()
        .filter(|record| record.stage == "pass" && record.fuzz.is_some())
        .collect();
    let held = fuzzed.iter().filter(|record| !record.fails_fuzz()).count();
    (held, fuzzed.len())
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
    fn fuzz_counts_leave_out_passes_without_fuzzed_inputs() {
        let record = |stage: &'static str, fuzz: Option<&'static str>| Record {
            name: String::new(),
            group: String::new(),
            stage,
            detail: String::new(),
            warnings: BTreeMap::new(),
            metrics: None,
            fuzz: fuzz.map(|stage| (stage, String::new())),
            seconds: 0.0,
        };
        let records = [
            record("pass", None),
            record("pass", Some("pass")),
            record("pass", Some("mismatch")),
            record("mismatch", None),
        ];
        assert_eq!(fuzz_counts(&records), (1, 2));
    }
}
