//! Collects a practice pool: single-file C programs from a public source that
//! is neither TRACTOR nor CodeNet, for a driver to steer by. The pool is no
//! benchmark set: no gate evaluates it and no result is reported on it. A
//! program joins when it needs nothing but its own run, without arguments or
//! input, and that run is a valid reference by the checks of the sets. Only
//! counts are printed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use eval::{Input, References};

const USAGE: &str = "usage: pool TAG SOURCE_DIR POOL_DIR [--jobs N]\n\
     Copies the C programs under SOURCE_DIR that are valid references into\n\
     POOL_DIR as gN_TAG_PATH.c. The programs are third-party code and run\n\
     here: call it through loop/cargo, with both directories in the checkout.";

/// Headers of what a pool program may not reach: the network, other
/// processes, files, the terminal and the clock. The programs run without a
/// sandbox whenever they are evaluated, so they must need none.
const HEADERS: [&str; 19] = [
    "sys/",
    "netinet/",
    "arpa/",
    "netdb.h",
    "unistd.h",
    "fcntl.h",
    "dirent.h",
    "pthread.h",
    "threads.h",
    "signal.h",
    "setjmp.h",
    "termios.h",
    "time.h",
    "omp.h",
    "windows.h",
    "winsock",
    "conio.h",
    "GL/",
    "SDL",
];
/// Library functions that do the same without one of those headers.
const CALLS: [&str; 10] = [
    "fopen", "freopen", "tmpfile", "remove", "rename", "system", "popen", "getenv", "time", "clock",
];

struct Args {
    tag: String,
    source: PathBuf,
    pool: PathBuf,
    jobs: usize,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Option<Args> {
    let (mut positional, mut jobs) = (Vec::new(), 4);
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--jobs" => jobs = rest.next()?.parse().ok().filter(|&n| n > 0)?,
            _ if arg.starts_with('-') => return None,
            _ => positional.push(arg),
        }
    }
    let [tag, source, pool] = <[String; 3]>::try_from(positional).ok()?;
    let short = !tag.is_empty() && tag.len() <= 8;
    (short && tag.chars().all(|c| c.is_ascii_lowercase())).then(|| Args {
        tag,
        source: source.into(),
        pool: pool.into(),
        jobs,
    })
}

fn main() -> ExitCode {
    let Some(args) = parse_args(std::env::args().skip(1)) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match collect(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn collect(args: &Args) -> Result<(), String> {
    let mut files = Vec::new();
    c_files(&args.source, &mut files)?;
    files.sort();
    let scratch = eval::Scratch::new("pool")?;
    let next = AtomicUsize::new(0);
    let checked = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..args.jobs {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(at) else { break };
                    let relative = file.strip_prefix(&args.source).unwrap_or(file);
                    let stem = stem(relative);
                    let outcome = check(file, &stem, &scratch.join(at.to_string()));
                    checked.lock().unwrap().push((stem, file, outcome));
                }
            });
        }
    });
    let mut checked = checked.into_inner().unwrap();
    checked.sort_by(|a, b| a.1.cmp(b.1));

    std::fs::create_dir_all(&args.pool)
        .map_err(|err| format!("cannot create {}: {err}", args.pool.display()))?;
    let mut left_out: BTreeMap<String, usize> = BTreeMap::new();
    let mut groups: BTreeMap<u8, usize> = BTreeMap::new();
    let mut names = BTreeSet::new();
    for (stem, file, outcome) in checked {
        let group = match outcome {
            Ok(group) => group,
            Err(reason) => {
                *left_out.entry(reason).or_default() += 1;
                continue;
            }
        };
        let name = format!("g{group}_{}_{stem}", args.tag);
        // Two paths that differ only in what a name cannot hold.
        if !names.insert(name.clone()) {
            *left_out.entry("name_taken".to_owned()).or_default() += 1;
            continue;
        }
        let to = args.pool.join(format!("{name}.c"));
        std::fs::copy(file, &to).map_err(|err| format!("cannot write {}: {err}", to.display()))?;
        *groups.entry(group).or_default() += 1;
    }
    println!(
        "pool {}: {} of {} programs",
        args.tag,
        names.len(),
        files.len()
    );
    println!("left out: {left_out:?}");
    println!("groups: {groups:?}");
    Ok(())
}

/// The C files under `dir`, at any depth. Links are not followed.
fn c_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let cannot = |err| format!("cannot read {}: {err}", dir.display());
    for entry in std::fs::read_dir(dir).map_err(cannot)? {
        let entry = entry.map_err(cannot)?;
        let (path, kind) = (entry.path(), entry.file_type().map_err(cannot)?);
        if kind.is_dir() {
            c_files(&path, files)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "c") {
            files.push(path);
        }
    }
    Ok(())
}

/// The part of a pool program's name that says where it comes from: its path
/// in the source without the extension, in lower case, with `_` for every
/// run of other characters.
fn stem(relative: &Path) -> String {
    let path = relative.with_extension("");
    let mut stem = String::new();
    for c in path.to_string_lossy().chars() {
        if c.is_ascii_alphanumeric() {
            stem.push(c.to_ascii_lowercase());
        } else if !stem.ends_with('_') && !stem.is_empty() {
            stem.push('_');
        }
    }
    stem.trim_end_matches('_').to_owned()
}

/// The group of the program `file` if it may join the pool, or why not.
fn check(file: &Path, stem: &str, work: &Path) -> Result<u8, String> {
    let source = std::fs::read(file).map_err(|err| format!("unreadable: {}", err.kind()))?;
    let Ok(source) = String::from_utf8(source) else {
        return Err("not_utf8".to_owned());
    };
    if stem.is_empty() {
        return Err("no_name".to_owned());
    }
    if reaches_outside(&source) {
        return Err("reaches_outside".to_owned());
    }
    std::fs::create_dir_all(work).map_err(|err| format!("no_scratch: {}", err.kind()))?;
    // Under its own name, as the evaluator will build it.
    let c_file = work.join(format!("{stem}.c"));
    std::fs::write(&c_file, &source).map_err(|err| format!("no_scratch: {}", err.kind()))?;
    let references = References::build(&c_file, &[], work)?;
    references.check(&Input::default())?;
    let (group, _) = bench::classify(&c_file)?;
    Ok(group)
}

/// Whether the program includes a header or calls a function that reaches
/// beyond its own memory, standard input and standard output.
fn reaches_outside(source: &str) -> bool {
    let included = source
        .lines()
        .map(str::trim_start)
        .filter_map(|line| line.strip_prefix('#'))
        .map(str::trim_start)
        .filter_map(|line| line.strip_prefix("include"))
        .any(|header| HEADERS.iter().any(|name| header.contains(name)));
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let called = CALLS.iter().any(|name| {
        source.match_indices(name).any(|(at, _)| {
            let before = source[..at].chars().next_back();
            let after = source[at + name.len()..].trim_start().chars().next();
            !before.is_some_and(word) && after == Some('(')
        })
    });
    included || called
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_says_where_the_program_comes_from() {
        assert_eq!(
            stem(Path::new("sorting/Bubble Sort-2.c")),
            "sorting_bubble_sort_2"
        );
        assert_eq!(stem(Path::new("00042.c")), "00042");
        assert_eq!(stem(Path::new("__.c")), "");
    }

    #[test]
    fn programs_that_reach_outside_are_recognised() {
        for source in [
            "#include <sys/socket.h>\nint main(void) { return 0; }",
            "  #  include \"unistd.h\"\n",
            "int main(void) { FILE *f = fopen (\"a\", \"w\"); }",
            "int main(void) { return (int)time(0); }",
        ] {
            assert!(reaches_outside(source), "{source}");
        }
        for source in [
            "#include <stdio.h>\n#include <string.h>\nint main(void) { return 0; }",
            "int runtime(int x) { return my_time(x) + clocks; }",
            "// no sys/ header here\nint main(void) { return 0; }",
        ] {
            assert!(!reaches_outside(source), "{source}");
        }
    }

    #[test]
    fn only_valid_references_join_the_pool() {
        let scratch = eval::Scratch::new("pool-test").unwrap();
        let (source, pool) = (scratch.join("source"), scratch.join("pool"));
        std::fs::create_dir_all(source.join("lists")).unwrap();
        let write = |name: &str, text: &str| std::fs::write(source.join(name), text).unwrap();
        write(
            "lists/Sum.c",
            "#include <stdio.h>\nint main(void) { int a[3] = {1, 2, 3}; printf(\"%d\\n\", a[0] + a[2]); return 0; }\n",
        );
        write("no_main.c", "int f(int x) { return x + 1; }\n");
        write(
            "overflow.c",
            "int main(void) { int a[2] = {0}; volatile int i = 2; return a[i]; }\n",
        );
        write(
            "clock.c",
            "#include <time.h>\nint main(void) { return 0; }\n",
        );
        write("notes.txt", "not a program\n");
        let args = Args {
            tag: "t".to_owned(),
            source,
            pool: pool.clone(),
            jobs: 2,
        };
        collect(&args).unwrap();
        let kept: Vec<_> = eval::entries(&pool)
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept, ["g2_t_lists_sum.c"]);
    }

    #[test]
    fn a_tag_is_a_short_lower_case_word() {
        let parsed = |args: &[&str]| parse_args(args.iter().map(|arg| arg.to_string()));
        assert!(parsed(&["ct", "source", "pool"]).is_some());
        assert_eq!(parsed(&["ct", "s", "p", "--jobs", "8"]).unwrap().jobs, 8);
        for args in [
            &["g1_x", "s", "p"][..],
            &["", "s", "p"],
            &["ct", "s"],
            &["ct", "s", "p", "--jobs", "0"],
        ] {
            assert!(parsed(args).is_none(), "{args:?}");
        }
    }
}
