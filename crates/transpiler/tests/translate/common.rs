//! Runs a C program and its translation side by side.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn run(command: &mut Command, stdin: &str) -> Output {
    use std::io::Write;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Translates the C program, or returns what has no rule.
pub fn translate(name: &str, c: &str) -> Result<String, String> {
    let dir = scratch(name);
    let source = dir.join("program.c");
    std::fs::write(&source, c).unwrap();
    let translated = transpiler::translate_file(&source);
    std::fs::remove_dir_all(&dir).unwrap();
    translated
}

/// Translates the C program, compiles both and runs them on each list of
/// arguments with `stdin`. They must print the same and exit alike, and the
/// translation must compile as safe Rust without warnings, apart from
/// those about code the C program does not use. Returns the translation.
pub fn check(name: &str, c: &str, runs: &[&[&str]], stdin: &str) -> String {
    let dir = scratch(name);
    let source = dir.join("program.c");
    std::fs::write(&source, c).unwrap();
    let rust = transpiler::translate_file(&source).unwrap_or_else(|err| panic!("{err}"));
    let translation = dir.join("translation.rs");
    std::fs::write(&translation, &rust).unwrap();

    let compiled = Command::new(transpiler::CLANG)
        .arg(&source)
        .args(["-lm", "-o"])
        .arg(dir.join("c"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let compiled = Command::new("rustc")
        .args([
            "--edition",
            "2024",
            "-C",
            "overflow-checks=on",
            "-C",
            "debug-assertions=on",
        ])
        .args([
            "-F",
            "unsafe_code",
            "-D",
            "warnings",
            "-A",
            "dead_code",
            "-o",
        ])
        .arg(dir.join("rust"))
        .arg(&translation)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{rust}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    for args in runs {
        let c = run(Command::new(dir.join("c")).args(*args), stdin);
        let rust_run = run(Command::new(dir.join("rust")).args(*args), stdin);
        assert_eq!(
            (
                String::from_utf8_lossy(&rust_run.stdout),
                rust_run.status.code()
            ),
            (String::from_utf8_lossy(&c.stdout), c.status.code()),
            "with {args:?}: {}\n{rust}",
            String::from_utf8_lossy(&rust_run.stderr)
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
    rust
}

/// Asserts that the translation holds each of the lines.
pub fn assert_lines(rust: &str, lines: &[&str]) {
    for line in lines {
        assert!(
            rust.lines().any(|found| found.trim() == *line),
            "no line `{line}` in\n{rust}"
        );
    }
}
