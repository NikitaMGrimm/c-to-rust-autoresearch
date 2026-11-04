//! Differential evaluator. A benchmark passes when its translation compiles as
//! safe Rust and, with overflow checks on, prints the same standard output and
//! exits with the same code as the C original.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Directory whose rust-toolchain.toml pins rustc and Clippy.
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const RUN_TIMEOUT: &str = "10";
const TRANSPILE_TIMEOUT: &str = "30";
/// Exit code of coreutils `timeout` when the limit is reached.
const TIMEOUT_EXIT: i32 = 124;
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
    fn timed_out(&self) -> bool {
        self.exit == Some(TIMEOUT_EXIT)
    }
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
    if rust.exit == Some(PANIC_EXIT) && c.exit != Some(PANIC_EXIT) {
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

fn stages(
    c_file: &Path,
    transpiler: &Path,
    work: &Path,
    warnings: &mut BTreeMap<String, u32>,
) -> (&'static str, String) {
    let c_program = work.join("c_program");
    let built = run(Command::new("clang-18")
        .args([
            "-g",
            "-fsanitize=address,undefined",
            "-fno-sanitize-recover=all",
            "-o",
        ])
        .arg(&c_program)
        .arg(c_file)
        .arg("-lm"));
    if built.exit != Some(0) {
        return (
            "c_invalid",
            format!("C build failed:\n{}", tail(&built.stderr)),
        );
    }
    let c = run(Command::new("timeout").arg(RUN_TIMEOUT).arg(&c_program));
    if c.timed_out() {
        return ("c_invalid", "C program timed out".to_owned());
    }
    if c.exit == Some(SANITIZER_EXIT) {
        return (
            "c_invalid",
            format!("sanitizer error:\n{}", tail(&c.stderr)),
        );
    }

    let rust_file = work.join("translation.rs");
    let transpiled = run(Command::new("timeout")
        .arg(TRANSPILE_TIMEOUT)
        .arg(transpiler)
        .arg(c_file)
        .arg("-o")
        .arg(&rust_file));
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
        return ("compile_fail", errors);
    }
    let rust = run(Command::new("timeout").arg(RUN_TIMEOUT).arg(&rust_program));
    let unsafe_uses = warnings.get("unsafe_code").copied().unwrap_or(0);
    classify(&c, &rust, unsafe_uses)
}

fn compile(rust_file: &Path, program: &Path) -> Run {
    run(Command::new("clippy-driver")
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
        .current_dir(REPO))
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

fn run(command: &mut Command) -> Run {
    let sanitizers = format!("exitcode={SANITIZER_EXIT}:detect_leaks=0");
    match command
        .env("ASAN_OPTIONS", &sanitizers)
        .env("UBSAN_OPTIONS", format!("exitcode={SANITIZER_EXIT}"))
        .output()
    {
        Ok(output) => Run {
            stdout: output.stdout,
            stderr: output.stderr,
            exit: output.status.code(),
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
        assert_eq!(classify(&ran("", 0), &ran("", PANIC_EXIT), 0).0, "panic");
        assert_eq!(
            classify(&ran("", 0), &ran("", TIMEOUT_EXIT), 0).0,
            "timeout"
        );
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
