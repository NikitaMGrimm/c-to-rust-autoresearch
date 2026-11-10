//! Evaluating translations: the stages a translation passes through and the
//! record of where it stopped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::files::{REPO, Scratch};
use crate::idioms::idiom_counts;
use crate::reference::{build_c, c_problem};
use crate::run::{OUTPUT_LIMIT, Run, run_in_place, run_program, tail};
use crate::spec::{Input, Spec, fuzzed_inputs, spec};
use crate::translate::{Translator, compile, diagnostics, translate};

/// Exit code of a Rust program that panicked.
const PANIC_EXIT: i32 = 101;

/// How far a translation got. Every stage but `Pass` is a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The C program or its spec cannot serve as a reference.
    CInvalid,
    TranspileFail,
    CompileFail,
    Timeout,
    Panic,
    Mismatch,
    /// Correct output, but the translation uses `unsafe`.
    Unsafe,
    Pass,
}

impl Stage {
    /// The name in reports and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CInvalid => "c_invalid",
            Self::TranspileFail => "transpile_fail",
            Self::CompileFail => "compile_fail",
            Self::Timeout => "timeout",
            Self::Panic => "panic",
            Self::Mismatch => "mismatch",
            Self::Unsafe => "unsafe",
            Self::Pass => "pass",
        }
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// Result for one benchmark program.
#[derive(Debug, Clone)]
pub struct Record {
    pub name: String,
    pub group: String,
    pub stage: Stage,
    pub detail: String,
    /// Compiler and Clippy warnings by lint name.
    pub warnings: BTreeMap<String, u32>,
    /// Idiomaticity counts of a translation that compiled.
    pub metrics: Option<BTreeMap<&'static str, u32>>,
    /// Stage and detail on the fuzzed inputs, for a translation that passed
    /// and a program that has them.
    pub fuzz: Option<(Stage, String)>,
    pub seconds: f64,
}

impl Record {
    /// A pass that does not hold on the program's fuzzed inputs.
    pub fn fails_fuzz(&self) -> bool {
        self.stage == Stage::Pass
            && self
                .fuzz
                .as_ref()
                .is_some_and(|(stage, _)| *stage != Stage::Pass)
    }

    /// A pass that neither switches lints off, gives up freeing memory nor
    /// keeps scalars as bytes, as a translation over an emulated C memory does.
    pub fn strict(&self) -> bool {
        self.stage == Stage::Pass
            && self.metrics.as_ref().is_some_and(|metrics| {
                ["lint_allows", "leaks", "byte_casts"]
                    .iter()
                    .all(|name| metrics.get(name) == Some(&0))
            })
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "group": self.group,
            "stage": self.stage.as_str(),
            "strict": self.strict(),
            "detail": self.detail,
            "warnings": self.warnings,
            "metrics": self.metrics,
            "fuzz": self.fuzz.as_ref().map(|(stage, detail)| {
                serde_json::json!({"stage": stage.as_str(), "detail": detail})
            }),
            "seconds": self.seconds,
        })
    }
}

/// Decides the stage of a translation that compiled, given both runs.
pub(crate) fn classify(c: &Run, rust: &Run, unsafe_uses: u32) -> (Stage, String) {
    if rust.timed_out() {
        return (Stage::Timeout, String::new());
    }
    // A C program may exit with 101 too; only the panic message tells them apart.
    if rust.exit == Some(PANIC_EXIT)
        && String::from_utf8_lossy(&rust.stderr).contains("panicked at")
    {
        return (Stage::Panic, tail(&rust.stderr));
    }
    if rust.overflowed {
        return (
            Stage::Mismatch,
            format!("output exceeds {OUTPUT_LIMIT} bytes"),
        );
    }
    if rust.stdout != c.stdout || rust.exit != c.exit {
        return (Stage::Mismatch, mismatch(c, rust));
    }
    if unsafe_uses > 0 {
        return (Stage::Unsafe, format!("{unsafe_uses} unsafe uses"));
    }
    (Stage::Pass, String::new())
}

/// Passes whose program has fuzzed inputs, and how many of them hold on them.
pub fn fuzz_counts(records: &[Record]) -> (usize, usize) {
    let fuzzed: Vec<&Record> = records
        .iter()
        .filter(|record| record.stage == Stage::Pass && record.fuzz.is_some())
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

/// Evaluates `programs` on `jobs` threads; records are sorted by name.
pub fn evaluate_all(
    programs: &[PathBuf],
    translator: &Translator,
    jobs: usize,
) -> Result<Vec<Record>, String> {
    let scratch = Scratch::new("eval")?;
    for program in programs {
        std::fs::create_dir(scratch.join(program.file_stem().unwrap_or_default()))
            .map_err(|err| format!("cannot create scratch directory: {err}"))?;
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
    let mut record = Record {
        name,
        group,
        stage: Stage::CInvalid,
        detail: String::new(),
        warnings: BTreeMap::new(),
        metrics: None,
        fuzz: None,
        seconds: 0.0,
    };
    (record.stage, record.detail) = stages(c_file, translator, work, &mut record);
    record.seconds = (start.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    record
}

/// Names the input in a failure detail when the program has named inputs.
fn on_input(input: &Input, detail: String) -> String {
    if input.name.is_empty() {
        detail
    } else {
        format!("input {}: {detail}", input.name)
    }
}

/// Runs the stages in order and returns where the translation stopped. The
/// warnings, idiom counts and fuzz result go straight into `record`.
fn stages(
    c_file: &Path,
    translator: &Translator,
    work: &Path,
    record: &mut Record,
) -> (Stage, String) {
    let Spec { inputs, c_flags } = match spec(c_file) {
        Ok(spec) => spec,
        Err(err) => return (Stage::CInvalid, err),
    };
    for dir in [work.join("c"), work.join("rust")] {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            return (
                Stage::CInvalid,
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
                Stage::CInvalid,
                format!("cannot open {}: {err}", work.display()),
            );
        }
    };
    // The same file name in both directories gives both the same argv[0].
    let (c_program, rust_program) = (work.join("c/program"), rust_dir.join("program"));
    let built = build_c(c_file, &c_program, &c_flags);
    if built.exit != Some(0) {
        return (
            Stage::CInvalid,
            format!("C build failed:\n{}", tail(&built.stderr)),
        );
    }
    let mut c_runs = Vec::new();
    for input in &inputs {
        let c = run_program(&c_program, input);
        if let Some(problem) = c_problem(&c) {
            return (Stage::CInvalid, on_input(input, problem));
        }
        c_runs.push(c);
    }

    let rust_file = rust_dir.join("translation.rs");
    if let Err(detail) = translate(c_file, translator, &rust_file) {
        return (Stage::TranspileFail, detail);
    }
    let compiled = compile(&rust_file, &rust_program);
    let errors = diagnostics(&compiled.stderr, &mut record.warnings);
    if compiled.exit != Some(0) {
        // Without JSON errors (spawn failure, timeout), show the raw output.
        let detail = if errors.is_empty() {
            tail(&compiled.stderr)
        } else {
            errors
        };
        return (Stage::CompileFail, detail);
    }
    record.metrics = std::fs::read_to_string(&rust_file)
        .ok()
        .and_then(|source| idiom_counts(&source));
    // Behaviour on every input comes first; `unsafe` is reported only after that.
    for (input, c) in inputs.iter().zip(&c_runs) {
        let rust = run_in_place(&rust_program, input, true);
        let (stage, detail) = classify(c, &rust, 0);
        if stage != Stage::Pass {
            return (stage, on_input(input, detail));
        }
    }
    let unsafe_uses = record.warnings.get("unsafe_code").copied().unwrap_or(0);
    let last = c_runs.last().expect("inputs are never empty");
    let result = classify(last, last, unsafe_uses);
    if result.0 == Stage::Pass {
        record.fuzz = fuzz_stage(c_file, &c_program, &rust_program);
    }
    result
}

/// How a passing translation does on the fuzzed inputs, if there are any.
/// They are a separate result, so the pass rule stays as it was.
fn fuzz_stage(c_file: &Path, c_program: &Path, rust_program: &Path) -> Option<(Stage, String)> {
    let inputs = match fuzzed_inputs(c_file) {
        Ok(inputs) if inputs.is_empty() => return None,
        Ok(inputs) => inputs,
        Err(err) => return Some((Stage::CInvalid, err)),
    };
    for input in &inputs {
        let c = run_program(c_program, input);
        if let Some(problem) = c_problem(&c) {
            return Some((Stage::CInvalid, on_input(input, problem)));
        }
        let rust = run_in_place(rust_program, input, true);
        let (stage, detail) = classify(&c, &rust, 0);
        if stage != Stage::Pass {
            return Some((stage, on_input(input, detail)));
        }
    }
    Some((Stage::Pass, String::new()))
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
        assert_eq!(classify(&ran("1\n", 3), &ran("1\n", 3), 0).0, Stage::Pass);
    }

    #[test]
    fn different_exit_code_is_a_mismatch() {
        assert_eq!(
            classify(&ran("1\n", 0), &ran("1\n", 1), 0).0,
            Stage::Mismatch
        );
    }

    #[test]
    fn exit_only_mismatch_says_output_is_identical() {
        let (_, detail) = classify(&ran("1\n", 3), &ran("1\n", 0), 0);
        assert!(detail.ends_with("output identical"), "{detail}");
    }

    #[test]
    fn missing_line_is_a_mismatch() {
        let (stage, detail) = classify(&ran("1\n2\n", 0), &ran("1\n", 0), 0);
        assert_eq!(stage, Stage::Mismatch);
        assert!(detail.contains("line 2"), "{detail}");
    }

    #[test]
    fn correct_unsafe_translation_fails() {
        assert_eq!(classify(&ran("1\n", 0), &ran("1\n", 0), 2).0, Stage::Unsafe);
    }

    #[test]
    fn panic_and_timeout_are_reported() {
        let mut panicked = ran("", PANIC_EXIT);
        panicked.stderr = b"thread 'main' panicked at src/main.rs:1:1".to_vec();
        assert_eq!(classify(&ran("", 0), &panicked, 0).0, Stage::Panic);
        assert_eq!(
            classify(&ran("", 0), &ran("", PANIC_EXIT), 0).0,
            Stage::Mismatch
        );
        assert_eq!(classify(&ran("", 0), &ran("", 124), 0).0, Stage::Timeout);
    }

    #[test]
    fn missing_final_newline_is_shown() {
        let (_, detail) = classify(&ran("hi\n", 0), &ran("hi", 0), 0);
        assert!(detail.contains(r#""hi\n""#), "{detail}");
    }

    #[test]
    fn fuzz_counts_leave_out_passes_without_fuzzed_inputs() {
        let record = |stage: Stage, fuzz: Option<Stage>| Record {
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
            record(Stage::Pass, None),
            record(Stage::Pass, Some(Stage::Pass)),
            record(Stage::Pass, Some(Stage::Mismatch)),
            record(Stage::Mismatch, None),
        ];
        assert_eq!(fuzz_counts(&records), (1, 2));
    }
}
