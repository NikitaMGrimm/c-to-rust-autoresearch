//! Collects a practice pool: single-file C programs from a public source that
//! is neither TRACTOR nor CodeNet, for a driver to steer by. The pool is no
//! benchmark set: no gate evaluates it and no result is reported on it. A
//! program joins when it needs nothing but its own run, without arguments or
//! input, and that run is a valid reference by the checks of the sets. Only
//! counts are printed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use eval::{Input, References};

const USAGE: &str = "usage: pool TAG SOURCE_DIR POOL_DIR [--jobs N]\n\
     Copies the C programs under SOURCE_DIR that are valid references into\n\
     POOL_DIR as gN_TAG_PATH.c. The programs are third-party code and run\n\
     here: call it through loop/cargo, with both directories in the checkout.";

/// The C library's functions that a pool program may call, beside those it
/// defines: they reach nothing but its own memory, standard input and the
/// standard streams. The evaluator runs C programs without a sandbox, so a
/// pool program must need none. A function that takes a `FILE *` is here
/// because nothing here opens a file, so only the standard streams exist.
/// The clock and the environment are left out as well: a run must repeat.
#[rustfmt::skip]
const LIBRARY: &[&str] = &[
    // stdio.h
    "printf", "fprintf", "sprintf", "snprintf", "vprintf", "vfprintf", "vsprintf", "vsnprintf",
    "puts", "fputs", "putchar", "putc", "fputc", "fwrite", "fflush", "scanf", "fscanf", "sscanf",
    "getchar", "getc", "fgetc", "ungetc", "fgets", "fread", "getline", "feof", "ferror",
    "clearerr", "perror",
    // stdlib.h
    "malloc", "calloc", "realloc", "free", "alloca", "abs", "labs", "llabs", "div", "ldiv",
    "lldiv", "atoi", "atol", "atoll", "atof", "strtol", "strtoul", "strtoll", "strtoull",
    "strtod", "strtof", "strtold", "qsort", "bsearch", "rand", "srand", "exit", "abort",
    // string.h
    "strlen", "strcpy", "strncpy", "strcat", "strncat", "strcmp", "strncmp", "strchr", "strrchr",
    "strstr", "strtok", "strspn", "strcspn", "strpbrk", "strdup", "strndup", "memcpy", "memmove",
    "memset", "memcmp", "memchr",
    // ctype.h, and what glibc's macros for it, for errno and for assert call
    "isalpha", "isdigit", "isalnum", "isspace", "isupper", "islower", "ispunct", "isxdigit",
    "isprint", "iscntrl", "isgraph", "isblank", "toupper", "tolower", "__ctype_b_loc",
    "__ctype_tolower_loc", "__ctype_toupper_loc", "__errno_location", "__assert_fail",
    // setjmp.h
    "setjmp", "_setjmp", "longjmp",
    // Compiler builtins without a library name, after `__builtin_`
    "va_start", "va_end", "va_copy", "expect", "unreachable", "trap", "constant_p", "clz", "ctz",
    "popcount", "bswap16", "bswap32", "bswap64", "isnan", "isinf", "isfinite", "signbit",
    "huge_val", "inf", "nan",
];
/// math.h, each also with the suffixes `f` and `l`.
#[rustfmt::skip]
const MATHS: &[&str] = &[
    "sqrt", "cbrt", "pow", "exp", "exp2", "log", "log2", "log10", "sin", "cos", "tan", "asin",
    "acos", "atan", "atan2", "sinh", "cosh", "tanh", "fabs", "floor", "ceil", "round", "trunc",
    "fmod", "fmax", "fmin", "hypot", "modf", "frexp", "ldexp",
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
    for (stem, _, outcome) in checked {
        let (group, source) = match outcome {
            Ok(program) => program,
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
        // The text that was checked, not the file, which may have changed.
        std::fs::write(&to, source)
            .map_err(|err| format!("cannot write {}: {err}", to.display()))?;
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

/// The group and the text of the program `file` if it may join the pool, or
/// why not.
fn check(file: &Path, stem: &str, work: &Path) -> Result<(u8, String), String> {
    let source = std::fs::read(file).map_err(|err| format!("unreadable: {}", err.kind()))?;
    let Ok(source) = String::from_utf8(source) else {
        return Err("not_utf8".to_owned());
    };
    if stem.is_empty() {
        return Err("no_name".to_owned());
    }
    std::fs::create_dir_all(work).map_err(|err| format!("no_scratch: {}", err.kind()))?;
    // Under its own name, as the evaluator will build it.
    let c_file = work.join(format!("{stem}.c"));
    std::fs::write(&c_file, &source).map_err(|err| format!("no_scratch: {}", err.kind()))?;
    // Before anything of it runs.
    if reaches_outside(&c_file)? {
        return Err("reaches_outside".to_owned());
    }
    let references = References::build(&c_file, &[], work)?;
    references.check(&Input::default())?;
    let (group, _) = bench::classify(&c_file)?;
    Ok((group, source))
}

/// Whether the program names a function that it does not define and that is
/// not in [`LIBRARY`] or [`MATHS`], or holds assembly. Clang's syntax tree
/// decides, so that neither a macro, a digraph nor a declaration of the
/// program's own hides a call.
fn reaches_outside(c_file: &Path) -> Result<bool, String> {
    let output = Command::new("clang-18")
        .args(["-Xclang", "-ast-dump=json", "-fsyntax-only", "-w"])
        .arg(c_file)
        .output()
        .map_err(|err| format!("cannot run clang-18: {err}"))?;
    let ast: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "c_build: no syntax tree".to_owned())?;
    let own: Vec<&serde_json::Value> = ast["inner"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|decl| {
            let loc = &decl["loc"];
            decl["isImplicit"] != true
                && loc["includedFrom"].is_null()
                && loc["expansionLoc"]["includedFrom"].is_null()
                && (loc["offset"].is_number() || loc["expansionLoc"]["offset"].is_number())
        })
        .collect();
    let has_body = |decl: &serde_json::Value| {
        let mut inner = decl["inner"].as_array().into_iter().flatten();
        inner.any(|node| node["kind"] == "CompoundStmt")
    };
    let defined: BTreeSet<&str> = own
        .iter()
        .filter(|decl| decl["kind"] == "FunctionDecl" && has_body(decl))
        .filter_map(|decl| decl["name"].as_str())
        .collect();
    Ok(own.iter().any(|decl| names_outside(decl, &defined)))
}

fn names_outside(node: &serde_json::Value, defined: &BTreeSet<&str>) -> bool {
    let kind = node["kind"].as_str().unwrap_or_default();
    if matches!(kind, "GCCAsmStmt" | "MSAsmStmt" | "FileScopeAsmDecl") {
        return true;
    }
    let referenced = &node["referencedDecl"];
    if kind == "DeclRefExpr" && referenced["kind"] == "FunctionDecl" {
        let name = referenced["name"].as_str().unwrap_or_default();
        if !defined.contains(name) && !in_library(name) {
            return true;
        }
    }
    let mut inner = node["inner"].as_array().into_iter().flatten();
    inner.any(|node| names_outside(node, defined))
}

fn in_library(name: &str) -> bool {
    let name = name.strip_prefix("__builtin_").unwrap_or(name);
    let maths = |name: &str| MATHS.contains(&name);
    let sized = |bits: &&str| {
        let rest = name.strip_prefix(*bits);
        rest.is_some_and(|rest| matches!(rest, "l" | "ll"))
    };
    LIBRARY.contains(&name)
        || maths(name)
        || name
            .strip_suffix(['f', 'l'])
            .is_some_and(|base| maths(base) || ["huge_val", "inf", "nan"].contains(&base))
        || ["clz", "ctz", "popcount"].iter().any(sized)
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
        let scratch = eval::Scratch::new("pool-outside").unwrap();
        let reaches = |source: &str| {
            let c_file = scratch.join("program.c");
            std::fs::write(&c_file, source).unwrap();
            reaches_outside(&c_file).unwrap()
        };
        for source in [
            "#include <stdio.h>\nint main(void) { FILE *f = (fopen)(\"a\", \"w\"); return !f; }",
            "#define _LARGEFILE64_SOURCE\n#include <stdio.h>\nint main(void) { return !fopen64(\"a\", \"w\"); }",
            "%:include <fcntl.h>\nint main(void) { return creat(\"a\", 0600); }",
            "#/**/include <sys/socket.h>\nint main(void) { return socket(2, 1, 0); }",
            "int socket(int, int, int);\nint main(void) { return socket(2, 1, 0); }",
            "int open();\nint main(void) { int (*f)() = open; return f(\"a\", 0); }",
            "#include <time.h>\nint main(void) { return (int)time(0); }",
            "#include <stdlib.h>\nint main(void) { return !getenv(\"HOME\"); }",
            "int main(void) { __asm__(\"nop\"); return 0; }",
        ] {
            assert!(reaches(source), "{source}");
        }
        let library = "#include <assert.h>\n#include <ctype.h>\n#include <errno.h>\n\
            #include <math.h>\n#include <stdarg.h>\n#include <stdio.h>\n#include <stdlib.h>\n\
            #include <string.h>\n\
            static int sum(int n, ...) { va_list a; va_start(a, n); int s = va_arg(a, int); va_end(a); return s + n; }\n\
            int main(void) { char *p = malloc(8); assert(p); strcpy(p, \"ab\"); errno = 0;\n\
            fprintf(stderr, \"%d %f %d\\n\", isalpha(p[0]), sqrtf(2.0f) + fabsl(1.0L), sum(1, 2));\n\
            if (isnan(pow(2, 3))) abort(); free(p); return __builtin_popcountll(7) - 3; }";
        for source in [
            library,
            "#include <time.h>\nint time_left(int x) { return x; }\nint main(void) { return time_left(0); }",
            "int open(int x) { return x; }\nint main(void) { return open(0); }",
        ] {
            assert!(!reaches(source), "{source}");
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
            "#include <time.h>\nint main(void) { return (int)time(0) & 0; }\n",
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
