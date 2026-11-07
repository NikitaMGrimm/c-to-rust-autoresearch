//! Mechanical C-to-Rust transpiler. Translation rules are added by the research
//! loop and listed in CATALOGUE.md; it reads Clang's syntax tree and emits Rust.

mod control;
mod stdio;
mod translate;
mod types;

use std::path::Path;
use std::process::Command;

/// Clang's JSON AST is not a stable interface, so every run uses this version.
pub const CLANG: &str = "clang-18";

/// Returns Clang's syntax tree of `input` as JSON.
pub fn clang_ast(input: &Path) -> Result<serde_json::Value, String> {
    let output = Command::new(CLANG)
        .args(["-Xclang", "-ast-dump=json", "-fsyntax-only"])
        .arg(input)
        .output()
        .map_err(|err| format!("cannot run {CLANG}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "{CLANG} rejected {}:\n{}",
            input.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|err| format!("invalid AST from {CLANG}: {err}"))
}

/// Translates a C syntax tree into Rust source.
pub fn translate(ast: &serde_json::Value) -> String {
    translate::program(ast)
}
