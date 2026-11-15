//! Marks private programs that nearly repeat a development program. A rule
//! learned on the development program passes such a twin almost for free, so
//! private results are reported with and without them. For a practice pool,
//! which a driver reads, it removes the pool's programs that nearly repeat a
//! private one instead.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: near-copies PRIVATE_DIR [--pool POOL_DIR]\n\
     Writes PRIVATE_DIR/near-copies.json and prints counts only. With --pool it\n\
     writes nothing there: it deletes the programs in POOL_DIR that nearly\n\
     repeat a private program or share a private program's name.";
/// Jaccard similarity of token 4-grams from which a private program counts as
/// a near copy of its closest development program. The TRACTOR sets have no
/// scores between 0.41 and 0.53.
const THRESHOLD: f64 = 0.5;
const SHINGLE: usize = 4;
/// The private sets; the transfer set only when there is one.
const SETS: [&str; 3] = ["validation", "held-out", "transfer"];

type Shingles = BTreeSet<Vec<String>>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let done = match args.as_slice() {
        [private] => mark(Path::new(private)),
        [private, flag, pool] if flag == "--pool" => clean(Path::new(private), Path::new(pool)),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn mark(private: &Path) -> Result<(), String> {
    let dev = shingled(&eval::dev_dir())?;
    let mut sets = serde_json::Map::new();
    for set in SETS {
        if set == "transfer" && !private.join(set).is_dir() {
            continue;
        }
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

/// Deletes the programs in `pool` that nearly repeat a program of a private
/// set, or have the name of one or of a variant, which the audit of a
/// transcript would take for the private program. It says how many, not
/// which, also where it fails: an error names no private program and no
/// program that it deleted or was about to.
fn clean(private: &Path, pool: &Path) -> Result<(), String> {
    let real = |dir: &Path| {
        dir.canonicalize()
            .map_err(|err| format!("cannot open {}: {err}", dir.display()))
    };
    let (private, pool) = (real(private)?, real(pool)?);
    // The programs of a private set would repeat themselves and be deleted.
    if pool.starts_with(&private) || private.starts_with(&pool) {
        return Err("the pool and the private directory must lie apart".to_owned());
    }
    let unread = |_| "cannot read a private set; is it sealed?".to_owned();
    let mut sets = Vec::new();
    let mut names = BTreeSet::new();
    for set in SETS {
        if set == "transfer" && !private.join(set).is_dir() {
            continue;
        }
        let programs = shingled(&private.join(set)).map_err(unread)?;
        if programs.is_empty() {
            return Err(format!("the private set {set} has no programs"));
        }
        names.extend(programs.keys().cloned());
        sets.extend(programs.into_values());
    }
    let variants = private.join("variants");
    if variants.is_dir() {
        for dir in eval::entries(&variants).map_err(unread)? {
            if dir.is_dir() {
                names.extend(shingled(&dir).map_err(unread)?.into_keys());
            }
        }
    }
    // A pool program's name may be a private one.
    let programs = shingled(&pool).map_err(|_| "cannot read a pool program".to_owned())?;
    let (mut copies, mut named) = (0, 0);
    for (name, shingles) in &programs {
        let copy = sets
            .iter()
            .any(|other| similarity(shingles, other) >= THRESHOLD);
        if !copy && !names.contains(name) {
            continue;
        }
        *if copy { &mut copies } else { &mut named } += 1;
        // The program last, so that a failure leaves it with its files.
        for extension in [eval::FUZZ_EXTENSION, "json", "c"] {
            match std::fs::remove_file(pool.join(name).with_extension(extension)) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                    return Err(format!(
                        "cannot delete a file of a pool program: {}",
                        err.kind()
                    ));
                }
                _ => {}
            }
        }
    }
    println!(
        "pool: deleted {copies} near-copies of private programs and {named} with a private name, of {} programs",
        programs.len()
    );
    Ok(())
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

/// The shingles of a program's own code: the `main` intake generates for a
/// library test case is the same in every such program, so it is left out.
fn shingles(source: &str) -> Shingles {
    let source = source
        .split(bench::GENERATED_MAIN)
        .next()
        .unwrap_or_default();
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
    use std::path::PathBuf;

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

    #[test]
    fn a_pool_loses_what_repeats_a_private_program_and_nothing_is_written() {
        let scratch = eval::Scratch::new("near-copies").unwrap();
        let (private, pool) = (scratch.join("private"), scratch.join("pool"));
        let program = "int main(void) { int total = 0; for (int i = 0; i < 9; i++) total += i * i; return total; }";
        let other = "#include <stdio.h>\nint main(void) { puts(\"a b c d e f g h\"); return 0; }";
        let write = |path: PathBuf, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(private.join("validation/g1_secret.c"), program);
        write(private.join("held-out/g1_other.c"), "int f(void);");
        write(
            private.join("variants/held-out/g1_other__r.c"),
            "int g(void);",
        );
        write(
            pool.join("g1_p_copy.c"),
            &format!("// from elsewhere\n{program}"),
        );
        write(pool.join("g1_p_copy.json"), "{}");
        write(pool.join("g1_p_new.c"), other);
        write(pool.join("g1_other__r.c"), other);
        clean(&private, &pool).unwrap();
        let left: Vec<_> = eval::entries(&pool)
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["g1_p_new.c"]);
        assert!(!private.join("near-copies.json").exists());
        assert!(clean(&scratch.join("nothing"), &pool).is_err());

        // A private set is no pool, by its own path or by a link to it.
        let link = scratch.join("link");
        std::os::unix::fs::symlink(private.join("validation"), &link).unwrap();
        for inside in [private.join("validation"), link, scratch.to_path_buf()] {
            assert!(clean(&private, &inside).is_err(), "{}", inside.display());
        }
        assert!(private.join("validation/g1_secret.c").exists());
        // Without a set to compare with, nothing is deleted, and a file that
        // cannot be deleted is not named.
        write(pool.join("g1_p_copy.c"), program);
        std::fs::create_dir(pool.join("g1_p_copy.json")).unwrap();
        let error = clean(&private, &pool).unwrap_err();
        assert!(
            !error.contains("copy") && !error.contains("secret"),
            "{error}"
        );
        assert!(pool.join("g1_p_copy.c").exists());
        std::fs::remove_dir(pool.join("g1_p_copy.json")).unwrap();
        std::fs::create_dir(pool.join("g1_secret.c")).unwrap();
        let error = clean(&private, &pool).unwrap_err();
        assert!(!error.contains("secret"), "{error}");
        std::fs::remove_dir(pool.join("g1_secret.c")).unwrap();
        std::fs::remove_dir_all(private.join("held-out")).unwrap();
        let error = clean(&private, &pool).unwrap_err();
        assert!(!error.contains("secret"), "{error}");
        assert!(pool.join("g1_p_copy.c").exists());
    }

    #[test]
    fn a_generated_main_does_not_count() {
        let main = format!(
            "\n{}\nint main(int argc, char *argv[]) {{ return f(argc); }}\n",
            bench::GENERATED_MAIN
        );
        let one = shingles(&format!("int f(int x) {{ return x + 1; }}{main}"));
        let other = shingles(&format!("while (y < z) y--;{main}"));
        assert_eq!(similarity(&one, &other), 0.0);
    }
}
