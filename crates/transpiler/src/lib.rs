//! Mechanical C-to-Rust transpiler. `DESIGN.md` describes its stages and the
//! Rust it aims for, and `CATALOGUE.md` lists its translation rules.

pub mod c;
mod clang;
pub mod facts;
mod lower;
mod names;
mod normalise;
mod rust;

use std::path::Path;
use std::process::Command;

/// Clang's JSON AST is not a stable interface, so every run uses this version.
pub const CLANG: &str = "clang-18";

/// What the transpiler has no rule for, and where in the C file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    pub what: String,
    /// The offset in the C file, or 0 when unknown.
    pub at: u32,
}

impl Unsupported {
    pub fn new(what: impl Into<String>, at: u32) -> Self {
        Unsupported {
            what: what.into(),
            at,
        }
    }
}

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

/// Reads a C syntax tree into the tree the rules work on.
pub fn read(ast: &serde_json::Value) -> Result<c::Program, Unsupported> {
    clang::program(ast)
}

/// Translates a C syntax tree into Rust source, or says what in it has no
/// rule. There is no fallback: a program is translated as `DESIGN.md`
/// describes or not at all.
pub fn translate(ast: &serde_json::Value) -> Result<String, Unsupported> {
    let mut program = clang::program(ast)?;
    let facts = facts::Facts::of(&program);
    normalise::narrow(&mut program, &facts);
    let facts = facts::Facts::of(&program);
    let file = lower::program(&program, &facts)?;
    Ok(rust::print(&file))
}

/// Translates the C file `input`. An error names the line of what has no
/// rule.
pub fn translate_file(input: &Path) -> Result<String, String> {
    let ast = clang_ast(input)?;
    translate(&ast).map_err(|unsupported| {
        let source = std::fs::read(input).unwrap_or_default();
        let before = source.get(..unsupported.at as usize).unwrap_or_default();
        let line = before.iter().filter(|&&byte| byte == b'\n').count() + 1;
        match unsupported.at {
            0 => format!("unsupported: {}", unsupported.what),
            _ => format!("unsupported: {}, at line {line}", unsupported.what),
        }
    })
}
