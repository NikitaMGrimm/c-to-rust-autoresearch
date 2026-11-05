//! Marks private programs that nearly repeat a development program. A rule
//! learned on the development program passes such a twin almost for free, so
//! private results are reported with and without them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: near-copies PRIVATE_DIR\n\
     Writes PRIVATE_DIR/near-copies.json and prints counts only.";
/// Jaccard similarity of token 4-grams from which a private program counts as
/// a near copy of its closest development program. The TRACTOR sets have no
/// scores between 0.41 and 0.53.
const THRESHOLD: f64 = 0.5;
const SHINGLE: usize = 4;
const SETS: [&str; 2] = ["validation", "held-out"];
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

type Shingles = BTreeSet<Vec<String>>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [private] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match mark(Path::new(private)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn mark(private: &Path) -> Result<(), String> {
    let dev = shingled(&Path::new(REPO).join("bench/dev"))?;
    let mut sets = serde_json::Map::new();
    for set in SETS {
        let programs = shingled(&private.join(set))?;
        let mut copies = BTreeMap::new();
        for (name, shingles) in &programs {
            let closest = dev
                .iter()
                .map(|(dev_name, dev_shingles)| (similarity(shingles, dev_shingles), dev_name))
                .max_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((score, dev_name)) = closest
                && score >= THRESHOLD
            {
                let score = (score * 100.0).round() / 100.0;
                copies.insert(
                    name.clone(),
                    serde_json::json!({"dev": dev_name, "similarity": score}),
                );
            }
        }
        println!("{set}: {} of {} programs", copies.len(), programs.len());
        sets.insert(set.to_owned(), serde_json::json!(copies));
    }
    let json = serde_json::json!({"threshold": THRESHOLD, "sets": sets});
    let path = private.join("near-copies.json");
    std::fs::write(&path, format!("{json:#}\n"))
        .map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn shingled(dir: &Path) -> Result<BTreeMap<String, Shingles>, String> {
    eval::c_programs(dir)?
        .iter()
        .map(|path| {
            let source = std::fs::read(path)
                .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            Ok((
                name.into_owned(),
                shingles(&String::from_utf8_lossy(&source)),
            ))
        })
        .collect()
}

fn shingles(source: &str) -> Shingles {
    tokens(source)
        .windows(SHINGLE)
        .map(<[String]>::to_vec)
        .collect()
}

fn similarity(a: &Shingles, b: &Shingles) -> f64 {
    let union = a.union(b).count().max(1);
    a.intersection(b).count() as f64 / union as f64
}

/// Identifiers, digit runs and single other characters, without comments.
/// Strings are not special: the measure only compares programs.
fn tokens(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let rest = &chars[i..];
        let end = if rest.starts_with(&['/', '*']) {
            let close = rest[2..].windows(2).position(|pair| pair == ['*', '/']);
            i + close.map_or(rest.len(), |at| at + 4)
        } else if rest.starts_with(&['/', '/']) {
            i + rest.iter().position(|&c| c == '\n').unwrap_or(rest.len())
        } else if chars[i].is_whitespace() {
            i + 1
        } else {
            let word = |c: char| c.is_alphanumeric() || c == '_';
            let len = if chars[i].is_alphabetic() || chars[i] == '_' {
                rest.iter().position(|&c| !word(c)).unwrap_or(rest.len())
            } else if chars[i].is_ascii_digit() {
                rest.iter()
                    .position(|c| !c.is_ascii_digit())
                    .unwrap_or(rest.len())
            } else {
                1
            };
            tokens.push(rest[..len].iter().collect());
            i + len
        };
        i = end;
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_skipped_and_numbers_split_from_suffixes() {
        let source = "int a1 = 10u; /* x * y */ b // c\n+= 2;";
        let expected = ["int", "a1", "=", "10", "u", ";", "b", "+", "=", "2", ";"];
        assert_eq!(tokens(source), expected);
        assert_eq!(tokens("a /* unclosed"), ["a"]);
    }

    #[test]
    fn similarity_is_one_for_copies_and_zero_for_unrelated_programs() {
        let program = shingles("int main(void) { return f(1) + 2; }");
        let same_with_comment = shingles("int main(void) { /* x */ return f(1) + 2; }");
        let other = shingles("while (x < y) x++ ; do_something ( x ) ;");
        assert_eq!(similarity(&program, &same_with_comment), 1.0);
        assert_eq!(similarity(&program, &other), 0.0);
        assert!(similarity(&shingles("a"), &shingles("a")) == 0.0);
    }
}
