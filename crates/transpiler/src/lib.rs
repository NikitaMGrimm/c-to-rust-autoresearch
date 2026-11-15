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

/// One refusal of a listing: what a program needs beside what stopped its
/// translation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocker {
    /// The stage that refused: `read`, `rewrite` or `rules`.
    pub stage: &'static str,
    /// The function that holds the construct, or nothing outside one.
    pub function: String,
    pub refusal: Unsupported,
    /// How often the function is refused for it.
    pub count: usize,
    /// Whether it names a declaration that an earlier refusal left out, so
    /// that it may only follow from that one.
    pub follows: bool,
}

/// The refusals of a listing, in the order they were found.
#[derive(Default)]
pub(crate) struct Listing {
    items: Vec<Blocker>,
    /// The names of the declarations that were refused and left out.
    holes: Vec<String>,
}

impl Listing {
    pub(crate) fn add(&mut self, stage: &'static str, function: &str, refusal: Unsupported) {
        let same =
            |item: &&mut Blocker| item.function == function && item.refusal.what == refusal.what;
        if let Some(item) = self.items.iter_mut().find(same) {
            item.count += 1;
            return;
        }
        let names = |hole: &String| refusal.what.contains(&format!("`{hole}`"));
        self.items.push(Blocker {
            stage,
            function: function.to_owned(),
            follows: self.holes.iter().any(names),
            refusal,
            count: 1,
        });
    }

    /// Notes that the declaration of `name` was refused: the program goes
    /// on without it.
    pub(crate) fn hole(&mut self, name: &str) {
        if !name.is_empty() && !self.holes.iter().any(|hole| hole == name) {
            self.holes.push(name.to_owned());
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
    let mut ast = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("invalid AST from {CLANG}: {err}"))?;
    clang::without_comments(&mut ast);
    Ok(ast)
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
    normalise::unions(&mut program)?;
    normalise::voids(&mut program)?;
    normalise::protos(&mut program)?;
    normalise::places(&mut program);
    normalise::join(&mut program);
    normalise::ends(&mut program);
    normalise::found(&mut program);
    let mut facts = facts::Facts::of(&program);
    if normalise::same(&mut program, &facts) {
        facts = facts::Facts::of(&program);
    }
    normalise::cursors(&mut program, &facts);
    let mut facts = facts::Facts::of(&program);
    if normalise::alias(&mut program, &facts) {
        facts = facts::Facts::of(&program);
    }
    normalise::narrow(&mut program, &facts);
    let facts = facts::Facts::of(&program);
    let file = lower::program(&program, &facts)?;
    Ok(rust::print(&file))
}

/// Lists what the program needs that has no rule: the refusal that stops
/// `translate`, first, and those that the stages find when each goes on
/// after a refusal without the declaration or the statement it refused. The
/// list is empty where `translate` gives a translation. It is a lower
/// bound; `DESIGN.md` says what it hides.
pub fn blockers(ast: &serde_json::Value) -> Vec<Blocker> {
    let mut listing = Listing::default();
    // The stages after a refusal work on a program with parts left out,
    // which none of them was written for: where one stops, the list ends.
    let stages = std::panic::AssertUnwindSafe(|| {
        let mut program = clang::listed(ast, &mut listing);
        if let Err(refusal) = normalise::unions(&mut program) {
            listing.add("rewrite", "", refusal);
        }
        if let Err(refusal) = normalise::voids(&mut program) {
            listing.add("rewrite", "", refusal);
        }
        if let Err(refusal) = normalise::protos(&mut program) {
            listing.add("rewrite", "", refusal);
        }
        normalise::places(&mut program);
        normalise::join(&mut program);
        normalise::ends(&mut program);
        normalise::found(&mut program);
        let mut facts = facts::Facts::of(&program);
        if normalise::same(&mut program, &facts) {
            facts = facts::Facts::of(&program);
        }
        normalise::cursors(&mut program, &facts);
        let mut facts = facts::Facts::of(&program);
        if normalise::alias(&mut program, &facts) {
            facts = facts::Facts::of(&program);
        }
        normalise::narrow(&mut program, &facts);
        let facts = facts::Facts::of(&program);
        lower::listed(&program, &facts, &mut listing);
    });
    if std::panic::catch_unwind(stages).is_err() {
        listing.add(
            "rules",
            "",
            Unsupported::new("the listing stopped here: a stage panicked", 0),
        );
    }
    listing.items
}

/// The refusal as `translate_file` prints it, with its line in `source`.
fn describe(unsupported: &Unsupported, source: &[u8]) -> String {
    let before = source.get(..unsupported.at as usize).unwrap_or_default();
    let line = before.iter().filter(|&&byte| byte == b'\n').count() + 1;
    match unsupported.at {
        0 => format!("unsupported: {}", unsupported.what),
        _ => format!("unsupported: {}, at line {line}", unsupported.what),
    }
}

/// Translates the C file `input`. An error names the line of what has no
/// rule.
pub fn translate_file(input: &Path) -> Result<String, String> {
    let ast = clang_ast(input)?;
    translate(&ast).map_err(|unsupported| {
        let source = std::fs::read(input).unwrap_or_default();
        describe(&unsupported, &source)
    })
}

/// Lists what the C file `input` needs that has no rule, a line for each
/// refusal of `blockers`: its stage, its function, the refusal as
/// `translate_file` prints it, how often the function meets it, and whether
/// it may only follow from an earlier one.
pub fn blockers_file(input: &Path) -> Result<Vec<String>, String> {
    let ast = clang_ast(input)?;
    let source = std::fs::read(input).unwrap_or_default();
    let line = |blocker: &Blocker| {
        let function = match blocker.function.as_str() {
            "" => "-",
            function => function,
        };
        let follows = if blocker.follows { "\tfollows" } else { "" };
        format!(
            "{}\t{function}\t{}\t{}{follows}",
            blocker.stage,
            describe(&blocker.refusal, &source),
            blocker.count
        )
    };
    Ok(blockers(&ast).iter().map(line).collect())
}
