//! Generates variants of every benchmark program, so a rule that only
//! memorises a program fails on its variants:
//!
//! - `r`: identifiers renamed consistently;
//! - `k`: renamed, and some integer constants changed;
//! - `i`: the original program on inputs with other numbers.
//!
//! The evaluator compares translations with the C program, so a variant needs
//! no expected output, only valid C reference runs. All variants go to the
//! private repository, including those of development programs: the loop
//! agent must not see them. Only counts are printed.

use bench::Rng;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: variants PRIVATE_DIR\n\
     PRIVATE_DIR is a checkout of the private holdout repository with a `seed` file.";
/// Attempts at a constants variant before giving up on a program.
const ATTEMPTS: u64 = 5;
const KEYWORDS: [&str; 48] = [
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "_Bool",
    "_Complex",
    "_Imaginary",
    "_Alignas",
    "_Alignof",
    "_Atomic",
    "_Generic",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
    "bool",
    "true",
    "false",
    "main",
];
/// Declarations whose names are renamed.
const NAMED: [&str; 9] = [
    "FunctionDecl",
    "VarDecl",
    "ParmVarDecl",
    "FieldDecl",
    "RecordDecl",
    "EnumDecl",
    "EnumConstantDecl",
    "TypedefDecl",
    "LabelStmt",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Identifier,
    Number,
}

/// A token of C source that variants may change, as a byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token {
    start: usize,
    end: usize,
    kind: Kind,
}

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [private] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match generate(private) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn generate(private: &Path) -> Result<(), String> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let seed = std::fs::read_to_string(private.join("seed"))
        .map_err(|err| format!("cannot read seed: {err}"))?;
    let seed = u64::from_str_radix(seed.trim(), 16).map_err(|_| "seed must be hexadecimal")?;
    let scratch = std::env::temp_dir().join(format!("c2r-variants-{}", std::process::id()));
    std::fs::create_dir(&scratch).map_err(|err| format!("cannot create scratch: {err}"))?;
    let sets = [
        ("dev", repo.join("bench/dev")),
        ("validation", private.join("validation")),
        ("held-out", private.join("held-out")),
    ];
    let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for (set, dir) in &sets {
        let out = private.join("variants").join(set);
        clear(&out)?;
        for c_file in c_files(dir)? {
            let stem = c_file.file_stem().unwrap_or_default().to_string_lossy();
            let work = scratch.join(set).join(stem.as_ref());
            for (kind, result) in variants(&c_file, seed, &work) {
                let label = match result {
                    Ok((source, spec)) => {
                        let name = format!("{stem}__{kind}");
                        write(&out, &name, &source, &spec, &c_file, kind)?;
                        "written"
                    }
                    Err(reason) => reason,
                };
                *counts.entry((set, label)).or_default() += 1;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    for ((set, label), count) in counts {
        println!("{set:>10}  {label:<20} {count}");
    }
    Ok(())
}

type Variant = Result<(String, eval::Spec), &'static str>;

/// The three variants of one program, each with only its valid inputs.
fn variants(c_file: &Path, seed: u64, work: &Path) -> Vec<(&'static str, Variant)> {
    let (source, spec) = match (std::fs::read_to_string(c_file), eval::spec(c_file)) {
        (Ok(source), Ok(spec)) => (source, spec),
        _ => return vec![("r", Err("unreadable"))],
    };
    let name = c_file.file_name().unwrap_or_default().to_string_lossy();
    let rng = |kind: &str| Rng(bench::mix(seed ^ bench::fnv(&format!("{name}/{kind}"))));
    let renamed = rename(c_file, &source, &mut rng("r"));
    let mut result = Vec::new();
    result.push((
        "r",
        renamed
            .clone()
            .and_then(|renamed| validate(&renamed, &spec, &work.join("r"))),
    ));
    let mut constants = rng("k");
    let changed = (0..ATTEMPTS).find_map(|attempt| {
        let changed = change_constants(renamed.as_ref().ok()?, &mut constants)?;
        validate(&changed, &spec, &work.join(format!("k{attempt}"))).ok()
    });
    result.push(("k", changed.ok_or("no_valid_constants")));
    let mut inputs = rng("i");
    let mutated = eval::Spec {
        inputs: spec
            .inputs
            .iter()
            .filter_map(|input| mutate_input(input, &mut inputs))
            .collect(),
        c_flags: spec.c_flags.clone(),
    };
    result.push((
        "i",
        if mutated.inputs.is_empty() {
            Err("no_numbers_in_inputs")
        } else {
            validate(&source, &mutated, &work.join("i"))
        },
    ));
    result
}

/// Keeps the inputs on which `source` is a valid C reference.
fn validate(source: &str, spec: &eval::Spec, work: &Path) -> Variant {
    std::fs::create_dir_all(work).map_err(|_| "scratch")?;
    let c_file = work.join("variant.c");
    std::fs::write(&c_file, source).map_err(|_| "scratch")?;
    let references =
        eval::References::build(&c_file, &spec.c_flags, work).map_err(|_| "does_not_build")?;
    let inputs: Vec<eval::Input> = spec
        .inputs
        .iter()
        .filter(|input| references.check(input).is_ok())
        .cloned()
        .collect();
    if inputs.is_empty() {
        return Err("no_valid_inputs");
    }
    Ok((
        source.to_owned(),
        eval::Spec {
            inputs,
            c_flags: spec.c_flags.clone(),
        },
    ))
}

/// Renames every identifier declared in the program itself, the same way
/// wherever it occurs, to a made-up name that nothing else uses.
fn rename(c_file: &Path, source: &str, rng: &mut Rng) -> Result<String, &'static str> {
    let output = Command::new("clang-18")
        .args(["-Xclang", "-ast-dump=json", "-fsyntax-only"])
        .arg(c_file)
        .output()
        .map_err(|_| "no_syntax_tree")?;
    let ast: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "no_syntax_tree")?;
    let mut declared = BTreeSet::new();
    for decl in ast["inner"].as_array().into_iter().flatten() {
        let loc = &decl["loc"];
        let own = decl["isImplicit"] != true
            && loc["includedFrom"].is_null()
            && loc["expansionLoc"]["includedFrom"].is_null()
            && (loc["offset"].is_number() || loc["expansionLoc"]["offset"].is_number());
        if own {
            names(decl, &mut declared);
        }
    }
    declared.extend(macro_names(source));
    // Names the headers use must keep their meaning.
    let headers = preprocessed_identifiers(&includes(source))?;
    declared.retain(|name| !headers.contains(name) && !KEYWORDS.contains(&name.as_str()));

    let mut taken: BTreeSet<String> = preprocessed_identifiers(source)?;
    taken.extend(KEYWORDS.iter().map(|word| word.to_string()));
    let mut new_names = BTreeMap::new();
    for name in &declared {
        let new_name = loop {
            let candidate = made_up_name(rng);
            if taken.insert(candidate.clone()) {
                break candidate;
            }
        };
        new_names.insert(name.as_str(), new_name);
    }
    Ok(replace(source, |token, text| {
        (token.kind == Kind::Identifier)
            .then(|| new_names.get(text).cloned())
            .flatten()
    }))
}

fn names(node: &serde_json::Value, declared: &mut BTreeSet<String>) {
    let kind = node["kind"].as_str().unwrap_or_default();
    if NAMED.contains(&kind)
        && let Some(name) = node["name"].as_str().filter(|name| !name.is_empty())
    {
        declared.insert(name.to_owned());
    }
    for child in node["inner"].as_array().into_iter().flatten() {
        names(child, declared);
    }
}

fn macro_names(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix('#')?.trim_start();
            let rest = rest.strip_prefix("define")?;
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

fn includes(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            line.trim_start()
                .strip_prefix('#')
                .is_some_and(|rest| rest.trim_start().starts_with("include"))
        })
        .map(|line| format!("{line}\n"))
        .collect()
}

/// Every identifier in `source` after preprocessing, headers included.
fn preprocessed_identifiers(source: &str) -> Result<BTreeSet<String>, &'static str> {
    let mut child = Command::new("clang-18")
        .args(["-E", "-x", "c", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "no_preprocessor")?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let source = source.to_owned();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        stdin.write_all(source.as_bytes())
    });
    let output = child.wait_with_output().map_err(|_| "no_preprocessor")?;
    let _ = writer.join();
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(tokens(&text)
        .into_iter()
        .filter(|token| token.kind == Kind::Identifier)
        .map(|token| text[token.start..token.end].to_owned())
        .collect())
}

/// A pronounceable name of two or three consonant-vowel syllables and a
/// final consonant, unlike any C keyword or library name.
fn made_up_name(rng: &mut Rng) -> String {
    const CONSONANTS: &[u8] = b"bdfgklmnprstvz";
    const VOWELS: &[u8] = b"aeiou";
    let syllables = 2 + rng.below(2);
    let mut name = String::new();
    for _ in 0..syllables {
        name.push(CONSONANTS[rng.below(CONSONANTS.len() as u64) as usize] as char);
        name.push(VOWELS[rng.below(VOWELS.len() as u64) as usize] as char);
    }
    name.push(CONSONANTS[rng.below(CONSONANTS.len() as u64) as usize] as char);
    name
}

/// Changes about half of the decimal integer constants of 2 or more by a
/// small amount. Returns `None` if there is nothing to change.
fn change_constants(source: &str, rng: &mut Rng) -> Option<String> {
    let mut changed = false;
    let result = replace(source, |token, text| {
        if token.kind != Kind::Number {
            return None;
        }
        let digits = text.trim_end_matches(['u', 'U', 'l', 'L']);
        let value: u64 = digits.parse().ok()?;
        // Leading zeros mean octal; 0 and 1 are usually flags or bounds.
        if digits.starts_with('0') || value < 2 || rng.below(2) == 0 {
            return None;
        }
        let delta = [-1i64, 1, 2, 3][rng.below(4) as usize];
        let new_value = (value as i64 + delta).max(2) as u64;
        changed |= new_value != value;
        Some(format!("{new_value}{}", &text[digits.len()..]))
    });
    changed.then_some(result)
}

/// The same input with about half of its numbers replaced by other numbers
/// of as many digits, or `None` if no number changed.
fn mutate_input(input: &eval::Input, rng: &mut Rng) -> Option<eval::Input> {
    let mut changed = false;
    let mut mutate = |text: &str| {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find(|c: char| c.is_ascii_digit()) {
            out.push_str(&rest[..start]);
            let digits: &str = {
                let after = &rest[start..];
                let end = after
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(after.len());
                &after[..end]
            };
            if rng.below(2) == 0 {
                out.push_str(digits);
            } else {
                let mut number = String::new();
                for i in 0..digits.len() {
                    let lowest = u64::from(i == 0 && digits.len() > 1);
                    let digit = lowest + rng.below(10 - lowest);
                    number.push(char::from(b'0' + digit as u8));
                }
                changed |= number != digits;
                out.push_str(&number);
            }
            rest = &rest[start + digits.len()..];
        }
        out.push_str(rest);
        out
    };
    let args = input.args.iter().map(|arg| mutate(arg)).collect();
    let stdin = mutate(&String::from_utf8_lossy(&input.stdin)).into_bytes();
    changed.then(|| eval::Input {
        name: format!("{}_m", input.name),
        args,
        stdin,
    })
}

/// Rebuilds `source`, replacing the tokens for which `change` returns text.
fn replace(source: &str, mut change: impl FnMut(&Token, &str) -> Option<String>) -> String {
    let mut out = String::with_capacity(source.len());
    let mut last = 0;
    for token in tokens(source) {
        if let Some(new) = change(&token, &source[token.start..token.end]) {
            out.push_str(&source[last..token.start]);
            out.push_str(&new);
            last = token.end;
        }
    }
    out.push_str(&source[last..]);
    out
}

/// Identifiers and numbers of C source, skipping comments, string and
/// character literals, and header names in `#include` lines.
fn tokens(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < bytes.len() {
        let byte = bytes[i];
        let rest = &source[i..];
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
        } else if let Some(comment) = rest.strip_prefix("/*") {
            i += comment.find("*/").map_or(rest.len(), |end| end + 4);
        } else if byte == b'"' || byte == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != byte && bytes[i] != b'\n' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else if byte == b'#' && line_start {
            let directive = rest[1..].trim_start();
            if directive.starts_with("include") {
                i += rest.find('\n').unwrap_or(rest.len());
            } else {
                i += 1;
            }
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let end = i + rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            tokens.push(Token {
                start: i,
                end,
                kind: Kind::Identifier,
            });
            i = end;
        } else if byte.is_ascii_digit()
            || (byte == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            // A preprocessing number: digits, letters, dots and exponent signs.
            let mut end = i + 1;
            while end < bytes.len() {
                let c = bytes[end];
                let exponent_sign =
                    (c == b'+' || c == b'-') && matches!(bytes[end - 1], b'e' | b'E' | b'p' | b'P');
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || exponent_sign {
                    end += 1;
                } else {
                    break;
                }
            }
            tokens.push(Token {
                start: i,
                end,
                kind: Kind::Number,
            });
            i = end;
        } else {
            i += source[i..].chars().next().map_or(1, char::len_utf8);
        }
        if i < bytes.len() {
            let previous = bytes[i.saturating_sub(1)];
            line_start =
                previous == b'\n' || (line_start && previous.is_ascii_whitespace() && i > 0);
        }
    }
    tokens
}

fn c_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .and_then(|entries| entries.map(|entry| entry.map(|e| e.path())).collect())
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    files.retain(|path| path.extension().is_some_and(|ext| ext == "c"));
    files.sort();
    Ok(files)
}

fn clear(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)
            .map_err(|err| format!("cannot clear {}: {err}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))
}

fn write(
    dir: &Path,
    name: &str,
    source: &str,
    spec: &eval::Spec,
    original: &Path,
    kind: &str,
) -> Result<(), String> {
    let inputs: Vec<_> = spec
        .inputs
        .iter()
        .map(|input| {
            serde_json::json!({
                "name": input.name,
                "args": input.args,
                "stdin": String::from_utf8_lossy(&input.stdin),
            })
        })
        .collect();
    let mut json = serde_json::json!({
        "source": format!("variant {kind} of {}", original.file_name().unwrap_or_default().to_string_lossy()),
        "inputs": inputs,
    });
    if !spec.c_flags.is_empty() {
        json["c_flags"] = serde_json::json!(spec.c_flags);
    }
    let base = dir.join(name);
    std::fs::write(base.with_extension("c"), source)
        .and_then(|()| std::fs::write(base.with_extension("json"), format!("{json:#}\n")))
        .map_err(|err| format!("cannot write {}: {err}", base.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = r#"#include <stdio.h>
#define LIMIT 10
// counter is mentioned in a comment
struct point { int x; int y; };
static int counter = 0;
static int bump(struct point *p, int step) { counter += step; return p->x + p->y + counter; }
int main(void) {
    struct point origin = { .x = 3, .y = 4 };
    int total = 0;
    for (int i = 0; i < LIMIT; i++) total += bump(&origin, 2);
    printf("counter=%d total=%d\n", counter, total);
    return 0;
}
"#;

    fn text(source: &str) -> Vec<&str> {
        tokens(source)
            .iter()
            .map(|token| &source[token.start..token.end])
            .collect()
    }

    #[test]
    fn tokens_skip_comments_strings_and_header_names() {
        let source = "#include <stdio.h>\n// a b\n/* c */ int d = 1e+5 + 'e' + .5; char *s = \"f\\\"g\";\n  #define H 0x1F\n";
        assert_eq!(
            text(source),
            ["int", "d", "1e+5", ".5", "char", "s", "define", "H", "0x1F"]
        );
    }

    #[test]
    fn renaming_changes_declared_names_only() {
        let dir = std::env::temp_dir().join(format!("variants-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c_file = dir.join("program.c");
        std::fs::write(&c_file, PROGRAM).unwrap();
        let renamed = rename(&c_file, PROGRAM, &mut Rng(1)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        for kept in [
            "printf",
            "main",
            "#include <stdio.h>",
            "counter is mentioned",
            "\"counter=%d",
        ] {
            assert!(renamed.contains(kept), "{kept} missing:\n{renamed}");
        }
        let words = text(&renamed);
        for gone in [
            "LIMIT", "point", "counter", "bump", "origin", "total", "step", "x", "y",
        ] {
            assert!(!words.contains(&gone), "{gone} kept:\n{renamed}");
        }
    }

    #[test]
    fn constants_change_but_flags_and_octal_stay() {
        let source = "int a = 0, b = 1, c = 010, d = 20u, e = 30;\n";
        let changed = (0..20)
            .find_map(|seed| change_constants(source, &mut Rng(seed)))
            .unwrap();
        let words = text(&changed);
        assert!(words.contains(&"0") && words.contains(&"1") && words.contains(&"010"));
        assert_ne!(changed, source);
        assert!(words.iter().any(|word| word.ends_with('u')), "{changed}");
    }

    #[test]
    fn mutated_inputs_keep_their_shape() {
        let input = eval::Input {
            name: "t".to_owned(),
            args: vec!["12".to_owned(), "abc".to_owned()],
            stdin: b"7 x 305\n".to_vec(),
        };
        let mutated = (0..20)
            .find_map(|seed| mutate_input(&input, &mut Rng(seed)))
            .unwrap();
        assert_eq!(mutated.name, "t_m");
        assert_eq!(mutated.args[0].len(), 2);
        assert!(!mutated.args[0].starts_with('0'));
        assert_eq!(mutated.args[1], "abc");
        let stdin = String::from_utf8(mutated.stdin).unwrap();
        assert!(stdin.contains(" x ") && stdin.ends_with('\n'), "{stdin}");
        let no_numbers = eval::Input {
            stdin: b"hello".to_vec(),
            ..eval::Input::default()
        };
        assert!(mutate_input(&no_numbers, &mut Rng(0)).is_none());
    }

    #[test]
    fn renamed_program_prints_the_same() {
        let dir = std::env::temp_dir().join(format!("variants-same-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c_file = dir.join("program.c");
        std::fs::write(&c_file, PROGRAM).unwrap();
        let renamed = rename(&c_file, PROGRAM, &mut Rng(2)).unwrap();
        let spec = eval::Spec {
            inputs: vec![eval::Input::default()],
            c_flags: Vec::new(),
        };
        let original = validate(PROGRAM, &spec, &dir.join("original"));
        let variant = validate(&renamed, &spec, &dir.join("variant"));
        let run = |work: &str| {
            eval::run_program(
                &dir.join(work).join("sanitized/program"),
                &eval::Input::default(),
            )
        };
        let (a, b) = (run("original"), run("variant"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(original.is_ok() && variant.is_ok());
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(a.stdout, b"counter=20 total=180\n");
    }
}
