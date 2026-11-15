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

/// The C library's symbols that a pool program's object files may need.
/// They reach nothing but the program's memory, standard input and the
/// standard streams: the evaluator runs C programs without a sandbox, so a
/// pool program should need none. A function that takes a `FILE *` is here
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
    // What the compilers call in their place
    "bcmp", "stpcpy", "mempcpy", "strnlen",
    // ctype.h, and what glibc's macros for it, for errno and for assert call
    "isalpha", "isdigit", "isalnum", "isspace", "isupper", "islower", "ispunct", "isxdigit",
    "isprint", "iscntrl", "isgraph", "isblank", "toupper", "tolower", "__ctype_b_loc",
    "__ctype_tolower_loc", "__ctype_toupper_loc", "__errno_location", "__assert_fail",
    // setjmp.h
    "setjmp", "_setjmp", "longjmp",
    // The standard streams, and what the compilers add
    "stdin", "stdout", "stderr", "__stack_chk_fail", "_GLOBAL_OFFSET_TABLE_", "__popcountdi2",
    "__popcountsi2", "__clzdi2", "__clzsi2", "__ctzdi2", "__ctzsi2", "__multi3", "__divti3",
    "__udivti3", "__modti3", "__umodti3",
];
/// How the compilers build a program before it serves as a reference, as
/// `eval::References` does, here up to the object file.
#[rustfmt::skip]
const BUILDS: [&[&str]; 4] = [
    &["clang-18", "-g", "-fsanitize=address,undefined", "-fno-sanitize-recover=all"],
    &["clang-18", "-g", "-O1", "-fsanitize=memory", "-fno-sanitize-recover=all", "-w"],
    &["clang-18", "-O2", "-w"],
    &["gcc", "-O2", "-w"],
];
/// What the sanitizers' own symbols start with.
#[rustfmt::skip]
const SANITIZERS: [&str; 6] = [
    "__asan_", "__ubsan_", "__msan_", "__sanitizer_", "__start_asan_", "__stop_asan_",
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
    if reaches_outside(&c_file, &source, work)? {
        return Err("reaches_outside".to_owned());
    }
    let references = References::build(&c_file, &[], work)?;
    references.check(&Input::default())?;
    let (group, _) = bench::classify(&c_file)?;
    Ok((group, source))
}

/// Whether an object file of the program, in any of the reference's builds,
/// needs a symbol beside those in [`LIBRARY`] and [`MATHS`], or its text
/// names assembly. The object file decides, not the source: a macro, a
/// declaration under another name or a function that is only called in one
/// build ends as a symbol there. This is a filter for programs from a
/// source that was not written against it, not a sandbox: assembly behind
/// pasted tokens passes it, and so does `main`'s third parameter, the
/// environment.
fn reaches_outside(c_file: &Path, source: &str, work: &Path) -> Result<bool, String> {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    if source.split(|c| !word(c)).any(|name| name.contains("asm")) {
        return Ok(true);
    }
    for (number, build) in BUILDS.iter().enumerate() {
        let object = work.join(format!("{number}.o"));
        let built = Command::new("timeout")
            .args(["60", build[0]])
            .args(&build[1..])
            .arg("-c")
            .arg("-o")
            .arg(&object)
            .arg(c_file)
            .output()
            .map_err(|err| format!("cannot run {}: {err}", build[0]))?;
        if !built.status.success() {
            return Err("c_build: object".to_owned());
        }
        let symbols = Command::new("nm")
            .args(["--undefined-only", "--format=posix"])
            .arg(&object)
            .output()
            .map_err(|err| format!("cannot run nm: {err}"))?;
        if !symbols.status.success() {
            return Err("c_build: symbols".to_owned());
        }
        let symbols = String::from_utf8_lossy(&symbols.stdout);
        let names = symbols.lines().filter_map(|line| line.split(' ').next());
        if !names.into_iter().all(in_library) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether an object file may need the symbol `name`.
fn in_library(name: &str) -> bool {
    if SANITIZERS.iter().any(|prefix| name.starts_with(prefix)) {
        return true;
    }
    // glibc's names for a function by the C standard in force, and its
    // checked forms.
    let name = ["__isoc99_", "__isoc23_", "_IO_"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .or_else(|| name.strip_prefix("__")?.strip_suffix("_chk"))
        .unwrap_or(name);
    let maths = |name: &str| MATHS.contains(&name);
    LIBRARY.contains(&name) || maths(name) || name.strip_suffix(['f', 'l']).is_some_and(maths)
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
            reaches_outside(&c_file, source, &scratch).unwrap()
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
            "extern char **environ;\nint main(void) { return !environ; }",
            "int main(void) { __asm__(\"nop\"); return 0; }",
            // Only one build has the call.
            "#ifndef __has_feature\n#define __has_feature(x) 0\n#endif\nint system(const char *);\n\
             int main(void) {\n#if __has_feature(address_sanitizer)\nreturn system(\"true\");\n#else\nreturn 0;\n#endif\n}",
            // The declarations seem to come from a header.
            "# 1 \"pretend.h\" 1\nint system(const char *);\nint main(void) { return system(\"true\"); }",
            // A library function's name for another symbol; assembly, too.
            "extern int puts(const char *) __asm__(\"system\");\nint main(void) { return puts(\"true\"); }",
            "extern char system;\nint main(void) { return ((int (*)(const char *))&system)(\"true\"); }",
            "extern int system(const char *);\n\
             int main(void) { char c __attribute__((cleanup(system))) = 0; return 0; }",
            // An inline definition gives no symbol of its own.
            "inline int system(const char *s) { return 0; }\nint main(void) { return system(\"true\"); }",
        ] {
            assert!(reaches(source), "{source}");
        }
        let library = "#define _GNU_SOURCE\n#include <assert.h>\n#include <ctype.h>\n#include <errno.h>\n\
            #include <math.h>\n#include <stdarg.h>\n#include <stdio.h>\n#include <stdlib.h>\n\
            #include <string.h>\n\
            static int sum(int n, ...) { va_list a; va_start(a, n); int s = va_arg(a, int); va_end(a); return s + n; }\n\
            int main(int argc, char **argv) { char *p = malloc(8); assert(p); strcpy(p, argv[0]); errno = 0;\n\
            char line[8]; if (fgets(line, sizeof line, stdin)) sscanf(line, \"%d\", &argc);\n\
            fprintf(stderr, \"%d %f %d %ld\\n\", isalpha(p[0]), sqrtf(argc) + fabsl(1.0L), sum(1, 2), strtol(p, 0, 10));\n\
            if (isnan(pow(argc, 3))) abort(); putchar(getchar()); free(p); return __builtin_popcountll(argc) - 3; }";
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
