//! Getting a translation: building and running the transpiler, or copying an
//! existing translation, then compiling it with Clippy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::files::REPO;
use crate::run::{COMPILE_TIMEOUT, DETAIL_LIMIT, Run, limited, run, tail};
use crate::sandbox::{sandboxed, toolchain};

const TRANSPILE_TIMEOUT: &str = "30";

const BUILD_TIMEOUT: &str = "600";

/// Where translations come from.
#[derive(Debug, Clone)]
pub enum Translator {
    /// The transpiler executable, run on each C program.
    Transpiler(PathBuf),
    /// A directory with an existing translation `NAME.rs` for each `NAME.c`.
    Existing(PathBuf),
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

/// Writes the translation of `c_file` to `rust_file`, or says why it could not.
pub(crate) fn translate(
    c_file: &Path,
    translator: &Translator,
    rust_file: &Path,
) -> Result<(), String> {
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

/// Compiles in the sandbox, since `include_str!` and the like read files.
pub(crate) fn compile(rust_file: &Path, program: &Path) -> Run {
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
pub(crate) fn diagnostics(stderr: &[u8], warnings: &mut BTreeMap<String, u32>) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_code_is_counted_despite_allow() {
        let dir = crate::Scratch::new("test").unwrap();
        let source = dir.join("translation.rs");
        std::fs::write(
            &source,
            "#![allow(unsafe_code)]\nfn main() {\n    let x = 1;\n    let p = &raw const x;\n    println!(\"{}\", unsafe { *p });\n}\n",
        )
        .unwrap();

        let compiled = compile(&source, &dir.join("program"));
        let mut warnings = BTreeMap::new();
        let errors = diagnostics(&compiled.stderr, &mut warnings);

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
