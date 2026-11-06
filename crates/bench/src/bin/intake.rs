//! Converts the standalone programs of DARPA's TRACTOR test corpus, and the
//! library test cases whose runner makes one plain call, into benchmarks and
//! splits them by family into development, validation and held-out sets. Only
//! names, groups and counts are printed, so running it does not show
//! validation or held-out programs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const CORPUS_URL: &str = "https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus";
const CORPUS_REVISION: &str = "37960ee08a7c644ee66e29855eca33903f702e67";
const USAGE: &str = "usage: intake CORPUS_DIR PRIVATE_DIR\n\
     CORPUS_DIR is fetched when missing; keep it outside this repository.\n\
     PRIVATE_DIR is a checkout of the private holdout repository with a `seed` file.";
/// Inputs whose C behaviour is undefined although the sanitizers miss it:
/// `atoi` of a value beyond `int` (C17 7.22.1.1), inside libc.
const KNOWN_UNDEFINED: [(&str, &str); 2] = [
    (
        "Public-Tests/B01_synthetic/009_stack_buffer_overflow",
        "test08_overflow_int1",
    ),
    (
        "Public-Tests/B02_synthetic/strcmp",
        "test_0092_permission_overflow",
    ),
];
const SPLITS: [(&str, f64); 3] = [("dev", 0.5), ("validation", 0.25), ("held-out", 0.25)];
const HEAP: [&str; 7] = [
    "malloc",
    "calloc",
    "realloc",
    "free",
    "strdup",
    "strndup",
    "aligned_alloc",
];
const STRING_FUNCTIONS: [&str; 18] = [
    "strlen", "strcpy", "strncpy", "strcat", "strncat", "strcmp", "strncmp", "strchr", "strrchr",
    "strstr", "strtok", "memcpy", "memmove", "memset", "memcmp", "sprintf", "snprintf", "strspn",
];
/// Functions that read input through pointer arguments.
const INPUT_PARSERS: [&str; 9] = [
    "scanf", "sscanf", "fscanf", "strtol", "strtoul", "strtoll", "strtoull", "strtod", "strtof",
];

struct Case {
    visibility: String,
    battery: String,
    name: String,
    dir: PathBuf,
}

impl Case {
    /// Library and executable forms of a test case form one family.
    fn family(&self) -> String {
        format!("{}/{}", self.battery, self.name.trim_end_matches("_lib"))
    }

    fn source(&self) -> String {
        format!("{}/{}/{}", self.visibility, self.battery, self.name)
    }
}

struct Program {
    family: String,
    source: String,
    file_name: String,
    c_file: PathBuf,
    licence: Option<PathBuf>,
    c_flags: Vec<String>,
    group: u8,
    signals: Vec<&'static str>,
    inputs: Vec<eval::Input>,
}

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [corpus, private] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match intake(corpus, private) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn intake(corpus: &Path, private: &Path) -> Result<(), String> {
    fetch(corpus)?;
    let seed = bench::seed(private)?;
    let inspected: BTreeSet<String> = read(&Path::new(eval::REPO).join("bench/inspected.txt"))?
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();

    // Holds the programs made from library test cases until they are written.
    let scratch = eval::Scratch::new("intake")?;
    let mut programs = Vec::new();
    let mut excluded: BTreeMap<String, String> = BTreeMap::new();
    let mut dropped: BTreeMap<&'static str, usize> = BTreeMap::new();
    // Executables first: a library form only counts for a family that has no
    // program from its executable.
    let (libraries, executables): (Vec<Case>, Vec<Case>) = cases(corpus)?
        .into_iter()
        .partition(|case| case.name.ends_with("_lib"));
    let mut converted = BTreeSet::new();
    for case in executables.iter().chain(&libraries) {
        match program(case, &converted, &scratch, &mut dropped) {
            Ok(program) => {
                converted.insert(program.family.clone());
                programs.push(program);
            }
            Err(reason) => {
                excluded.insert(case.source(), reason);
            }
        }
    }

    // Clearing the sets after a failure everywhere, e.g. without clang-18, would empty them.
    if programs.is_empty() {
        return Err("no test case converted; check excluded reasons".to_owned());
    }
    let names: BTreeSet<&str> = programs.iter().map(|p| p.file_name.as_str()).collect();
    if names.len() != programs.len() {
        return Err("two programs got the same file name".to_owned());
    }
    let split_file = private.join("split.json");
    let mut split: BTreeMap<String, String> = match std::fs::read(&split_file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|err| format!("split.json: {err}"))?,
        Err(_) => BTreeMap::new(),
    };
    assign(&programs, &inspected, seed, &mut split)?;

    let outputs = bench::sets(private);
    let mut resolved = BTreeSet::new();
    for (_, dir) in &outputs {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        let dir = dir
            .canonicalize()
            .map_err(|err| format!("cannot resolve {}: {err}", dir.display()))?;
        // A private set written into bench/dev would publish it.
        if !resolved.insert(dir.clone()) {
            return Err(format!("two sets resolve to {}", dir.display()));
        }
    }
    for (_, dir) in &outputs {
        clear(dir)?;
    }
    let mut counts: BTreeMap<(u8, &str), usize> = BTreeMap::new();
    for program in &programs {
        let set = split[&program.family].as_str();
        let (_, dir) = outputs.iter().find(|(name, _)| *name == set).unwrap();
        write_program(program, dir)?;
        *counts.entry((program.group, set)).or_default() += 1;
    }
    let json = |value: &serde_json::Value| format!("{value:#}\n");
    write(&split_file, &json(&serde_json::to_value(&split).unwrap()))?;
    let report = serde_json::json!({
        "corpus": format!("{CORPUS_URL} {CORPUS_REVISION}"),
        "programs": programs.iter().map(|p| serde_json::json!({
            "name": p.file_name, "source": p.source, "split": split[&p.family],
            "signals": p.signals, "inputs": p.inputs.len(),
        })).collect::<Vec<_>>(),
        "excluded": excluded,
        "dropped_inputs": dropped,
    });
    write(&private.join("intake.json"), &json(&report))?;

    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    for reason in excluded.values() {
        *reasons
            .entry(reason.split(':').next().unwrap())
            .or_default() += 1;
    }
    println!("excluded test cases: {reasons:?}");
    println!("dropped inputs: {dropped:?}");
    println!("group  dev  validation  held-out");
    for group in 1..=5 {
        let count = |set| counts.get(&(group, set)).copied().unwrap_or(0);
        println!(
            "g{group}  {:>8}  {:>10}  {:>8}",
            count("dev"),
            count("validation"),
            count("held-out")
        );
    }
    Ok(())
}

/// Fetches the pinned corpus revision when `dir` is missing, then checks it.
fn fetch(dir: &Path) -> Result<(), String> {
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .map_err(|err| format!("cannot run git: {err}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            Err(format!(
                "git {} failed:\n{}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    };
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|err| format!("cannot create corpus dir: {err}"))?;
        git(&["init", "-q"])?;
        git(&["fetch", "-q", "--depth", "1", CORPUS_URL, CORPUS_REVISION])?;
        git(&["checkout", "-q", "FETCH_HEAD"])?;
    }
    if !git(&["status", "--porcelain"])?.is_empty() {
        return Err(format!("{} has local changes", dir.display()));
    }
    let head = git(&["rev-parse", "HEAD"])?;
    if head != CORPUS_REVISION {
        return Err(format!(
            "{} is at {head}, expected {CORPUS_REVISION}",
            dir.display()
        ));
    }
    Ok(())
}

/// Test cases of batteries B01 and B02, public and formerly hidden.
fn cases(corpus: &Path) -> Result<Vec<Case>, String> {
    let mut cases = Vec::new();
    for visibility in ["Public-Tests", "Hidden-Tests"] {
        for battery in eval::entries(&corpus.join(visibility))? {
            let battery_name = file_name(&battery);
            if !(battery_name.starts_with("B01_") || battery_name.starts_with("B02_")) {
                continue;
            }
            for dir in eval::entries(&battery)? {
                if dir.join("test_case").is_dir() && dir.join("test_vectors").is_dir() {
                    cases.push(Case {
                        visibility: visibility.to_owned(),
                        battery: battery_name.clone(),
                        name: file_name(&dir),
                        dir,
                    });
                }
            }
        }
    }
    Ok(cases)
}

/// Converts a test case, or says why it cannot be used. A library form adds
/// nothing to a family in `converted`, which has a program already.
fn program(
    case: &Case,
    converted: &BTreeSet<String>,
    scratch: &Path,
    dropped: &mut BTreeMap<&'static str, usize>,
) -> Result<Program, String> {
    let library = case.name.ends_with("_lib");
    if library && converted.contains(&case.family()) {
        return Err("library".to_owned());
    }
    let test_case = case.dir.join("test_case");
    let sources = eval::entries(&test_case.join("src"))?;
    let [c_file] = sources.as_slice() else {
        return Err("multiple_files".to_owned());
    };
    // A library's header is inlined below; a program has none.
    if (!library && test_case.join("include").exists()) || !file_name(c_file).ends_with(".c") {
        return Err("multiple_files".to_owned());
    }
    if test_case.join("configurations.json").exists() {
        return Err("build_settings".to_owned());
    }
    let mut c_flags = Vec::new();
    for (command, args) in cmake_commands(&read(&test_case.join("CMakeLists.txt"))?) {
        let words: Vec<&str> = args.split_whitespace().collect();
        let allowed = match command.as_str() {
            "cmake_minimum_required" | "project" | "add_executable" => true,
            "add_library" | "target_include_directories" | "install" | "cmake_path" => library,
            // The evaluator always links the maths library.
            "target_link_libraries" => words
                .iter()
                .skip(1)
                .filter(|word| !matches!(**word, "PRIVATE" | "PUBLIC" | "INTERFACE"))
                .eq(["m"].iter()),
            "target_compile_options" => {
                let flags = words
                    .iter()
                    .skip(1)
                    .filter(|word| !matches!(**word, "PRIVATE" | "PUBLIC" | "INTERFACE"));
                c_flags.extend(flags.map(|flag| flag.to_string()));
                c_flags
                    .iter()
                    .all(|flag| eval::C_FLAGS.contains(&flag.as_str()))
            }
            _ => false,
        };
        if !allowed {
            return Err(format!("build_settings: {command}"));
        }
    }

    let runner = if library {
        let text = read(&case.dir.join("runner/src/main.rs"))?;
        let name = case.name.trim_end_matches("_lib");
        Some(parse_runner(&text, name).ok_or("library_runner")?)
    } else {
        None
    };
    // Cases of different batteries may share a name, and a program written
    // here is read again only once all cases are converted.
    let scratch_name = case.source().replace('/', "_");
    let c_file = match &runner {
        Some(runner) => {
            let program = scratch.join(format!("{scratch_name}.c"));
            let source = inline_headers(c_file, &test_case.join("include"))?;
            write(&program, &(source + &main_for(runner)))?;
            program
        }
        None => c_file.clone(),
    };
    let references = eval::References::build(&c_file, &c_flags, &scratch.join(&scratch_name))?;

    let mut inputs = Vec::new();
    for vector in eval::entries(&case.dir.join("test_vectors"))? {
        if !file_name(&vector).ends_with(".json") {
            continue;
        }
        let json: serde_json::Value = serde_json::from_str(&read(&vector)?)
            .map_err(|err| format!("vector_format: {}: {err}", vector.display()))?;
        let name = file_name(&vector).trim_end_matches(".json").to_owned();
        if KNOWN_UNDEFINED.contains(&(case.source().as_str(), name.as_str())) {
            *dropped.entry("known_undefined_behaviour").or_default() += 1;
            continue;
        }
        let converted = match &runner {
            Some(runner) => library_input(&json, &name, runner, &references),
            None => input(&json, &name, &references),
        };
        match converted {
            Ok(input) => inputs.push(input),
            Err(reason) => *dropped.entry(reason).or_default() += 1,
        }
    }
    if inputs.is_empty() {
        return Err("no_valid_inputs".to_owned());
    }
    let (group, signals) = classify(&c_file)?;
    let licence = test_case.join("LICENSE");
    Ok(Program {
        family: case.family(),
        source: case.source(),
        file_name: format!("g{group}_{}", case.name.trim_end_matches("_lib")),
        c_file,
        licence: licence.exists().then_some(licence),
        c_flags,
        group,
        signals,
        inputs,
    })
}

/// The commands of a CMake file as lower-case name and argument text. Arguments
/// may span lines; the corpus uses no parentheses inside them.
fn cmake_commands(text: &str) -> Vec<(String, String)> {
    let code: String = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    code.split(')')
        .filter_map(|command| {
            let (name, args) = command.split_once('(')?;
            Some((name.trim().to_lowercase(), args.trim().to_owned()))
        })
        .collect()
}

/// Turns a test vector into an input after checking that it is a valid C
/// reference run with TRACTOR's expected result.
fn input(
    vector: &serde_json::Value,
    name: &str,
    references: &eval::References,
) -> Result<eval::Input, &'static str> {
    let known = [
        "argv", "stdin", "stdout", "stderr", "rc", "note", "has_ub", "env",
    ];
    let fields = vector.as_object().ok_or("vector_format")?;
    if fields.keys().any(|key| !known.contains(&key.as_str())) {
        return Err("vector_format");
    }
    if fields.contains_key("has_ub") {
        return Err("marked_undefined_behaviour");
    }
    if vector["env"].as_object().is_some_and(|env| !env.is_empty()) {
        return Err("environment");
    }
    let stdout = &vector["stdout"];
    if stdout["is_regex"] == true {
        return Err("output_pattern");
    }
    let args = match &vector["argv"] {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(args) => args
            .iter()
            .map(|arg| arg.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or("vector_format")?,
        _ => return Err("vector_format"),
    };
    let stdin = match &vector["stdin"] {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::String(stdin) => stdin.as_bytes().to_vec(),
        _ => return Err("vector_format"),
    };
    let input = eval::Input {
        name: name.to_owned(),
        args,
        stdin,
    };
    let first = references.check(&input)?;
    let expected_exit = vector["rc"].as_i64().unwrap_or(0);
    let expected_stdout = stdout["pattern"].as_str();
    if first.exit != Some(expected_exit as i32)
        || expected_stdout.is_some_and(|out| out.as_bytes() != first.stdout)
    {
        return Err("c_differs_from_expected");
    }
    Ok(input)
}

/// What a library test case's runner does, where a C `main` can repeat it:
/// a state of scalars and strings, and one call of the library's symbol with
/// fields as arguments, whose result may go to a field.
#[derive(Debug, PartialEq)]
struct Runner {
    fields: Vec<(String, CType)>,
    symbol: String,
    result: Option<String>,
    args: Vec<Arg>,
}

/// An argument of the runner's call: a field, or a string field's first byte.
#[derive(Debug, PartialEq)]
enum Arg {
    Field(String),
    FirstByte(String),
}

/// A state field's type in C. A `bool` travels as an `int`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CType {
    Signed(&'static str),
    Unsigned(&'static str),
    Float,
    Double,
    Bool,
    String,
}

impl CType {
    fn of(rust: &str) -> Option<Self> {
        Some(match rust.rsplit("::").next()? {
            "i8" | "c_schar" => Self::Signed("signed char"),
            "c_char" => Self::Signed("char"),
            "i16" | "c_short" => Self::Signed("short"),
            "i32" | "c_int" => Self::Signed("int"),
            "c_long" => Self::Signed("long"),
            "i64" | "c_longlong" => Self::Signed("long long"),
            "u8" | "c_uchar" => Self::Unsigned("unsigned char"),
            "u16" | "c_ushort" => Self::Unsigned("unsigned short"),
            "u32" | "c_uint" => Self::Unsigned("unsigned int"),
            "c_ulong" => Self::Unsigned("unsigned long"),
            "u64" | "c_ulonglong" => Self::Unsigned("unsigned long long"),
            "usize" => Self::Unsigned("size_t"),
            "f32" | "c_float" => Self::Float,
            "f64" | "c_double" => Self::Double,
            "bool" => Self::Bool,
            "CString" => Self::String,
            _ => return None,
        })
    }

    /// The `printf` conversion that prints a value of this type; floats with
    /// enough digits to read them back exactly.
    fn format(self) -> &'static str {
        match self {
            Self::Signed("long") => "%ld",
            Self::Signed("long long") => "%lld",
            Self::Signed(_) | Self::Unsigned("unsigned char" | "unsigned short") | Self::Bool => {
                "%d"
            }
            Self::Unsigned("unsigned long") => "%lu",
            Self::Unsigned("unsigned long long") => "%llu",
            Self::Unsigned("size_t") => "%zu",
            Self::Unsigned(_) => "%u",
            Self::Float => "%.9g",
            Self::Double => "%.17g",
            Self::String => "%s",
        }
    }
}

/// Reads a runner written with TRACTOR's `harness!` macro, or returns `None`
/// when it does more than one plain call. A runner that names no symbol calls
/// the function named like its test case, `default_symbol`.
fn parse_runner(text: &str, default_symbol: &str) -> Option<Runner> {
    let text = &text[text.find("harness!")?..];
    let mut fields = Vec::new();
    for field in block(&text[text.find("state:")?..])?.split(',') {
        if field.trim().is_empty() {
            continue;
        }
        let (name, ty) = field.split_once(':')?;
        fields.push((name.trim().to_owned(), CType::of(ty.trim())?));
    }
    // Only a string literal names the function for certain.
    let symbol = match text.find("symbol:") {
        Some(at) => {
            let literal = text[at + "symbol:".len()..]
                .trim_start()
                .strip_prefix('"')?;
            literal[..literal.find('"')?].to_owned()
        }
        None => default_symbol.to_owned(),
    };
    let body: Vec<&str> = block(&text[text.find("fn run")?..])?
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect();
    let tokens = tokens(&body.join("\n"));
    let mut rest = tokens.as_slice();
    let ty = |name: &str| {
        fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, ty)| *ty)
    };
    let result = match rest {
        [this, dot, field, eq, tail @ ..] if this == "self" && dot == "." && eq == "=" => {
            rest = tail;
            if ty(field)? == CType::String {
                return None;
            }
            Some(field.clone())
        }
        _ => None,
    };
    // unsafe { (*SYMBOL)(ARGS) [;] } [;]
    let rest = rest.strip_suffix(&[";".to_owned()]).unwrap_or(rest);
    let rest = match rest {
        [unsafe_, open, inner @ .., close]
            if unsafe_ == "unsafe" && open == "{" && close == "}" =>
        {
            inner.strip_suffix(&[";".to_owned()]).unwrap_or(inner)
        }
        _ => return None,
    };
    let args = match rest {
        [a, b, c, d, e, args @ .., last]
            if [a, b, c, d, e].map(String::as_str) == ["(", "*", "SYMBOL", ")", "("]
                && last == ")" =>
        {
            args
        }
        _ => return None,
    };
    let mut parsed = Vec::new();
    for arg in args
        .split(|token| token == ",")
        .filter(|arg| !arg.is_empty())
    {
        // `as` casts to the parameter's type, which C does by itself. Only a
        // type may follow it, not arithmetic or a second cast.
        let arg = match arg.iter().position(|token| token == "as") {
            Some(at) => {
                let type_token = |token: &String| {
                    token == ":"
                        || token == "*"
                        || token != "as"
                            && token.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                };
                if !arg[at + 1..].iter().all(type_token) {
                    return None;
                }
                &arg[..at]
            }
            None => arg,
        };
        let words: Vec<&str> = arg.iter().map(String::as_str).collect();
        parsed.push(match words.as_slice() {
            ["self", ".", field] if ty(field)? != CType::String => Arg::Field(field.to_string()),
            ["self", ".", field, ".", "as_ptr", "(", ")"] if ty(field)? == CType::String => {
                Arg::Field(field.to_string())
            }
            ["self", ".", field, ".", "as_bytes", "(", ")", "[", "0", "]"]
                if ty(field)? == CType::String =>
            {
                Arg::FirstByte(field.to_string())
            }
            _ => return None,
        });
    }
    Some(Runner {
        fields,
        symbol,
        result,
        args: parsed,
    })
}

/// The text inside the first `{ ... }` of `text`, braces matched.
fn block(text: &str) -> Option<&str> {
    let start = text.find('{')? + 1;
    let mut depth = 1;
    for (at, c) in text[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => continue,
        }
        if depth == 0 {
            return Some(&text[start..start + at]);
        }
    }
    None
}

/// Words and single punctuation characters.
fn tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut in_word = false;
    for c in text.chars() {
        let word = c.is_ascii_alphanumeric() || c == '_';
        if word && in_word {
            tokens.last_mut().unwrap().push(c);
        } else if !c.is_whitespace() {
            tokens.push(c.to_string());
        }
        in_word = word;
    }
    tokens
}

/// The library's source with each of its own headers in place of the
/// `#include` that names it, so that the program is a single file. Lines end
/// in `\n`, also where a file of the library ends them in `\r\n`.
fn inline_headers(c_file: &Path, include: &Path) -> Result<String, String> {
    let headers = if include.exists() {
        eval::entries(include)?
    } else {
        Vec::new()
    };
    let mut source = String::new();
    for line in read(c_file)?.lines() {
        let quoted = line.trim_start().starts_with('#') && line.contains("include");
        match line.split('"').nth(1).filter(|_| quoted) {
            Some(name) => {
                let name = Path::new(name).file_name().unwrap_or_default();
                let header = headers
                    .iter()
                    .find(|header| header.file_name() == Some(name))
                    .ok_or("multiple_files")?;
                for line in read(header)?.lines() {
                    if line.contains("#include \"") {
                        return Err("multiple_files".to_owned());
                    }
                    source += line;
                    source.push('\n');
                }
            }
            None => {
                source += line;
                source.push('\n');
            }
        }
    }
    Ok(source)
}

/// A `main` that does what the runner does: it reads the input state from
/// the command line, one argument per field, makes the call and prints the
/// field that receives its result.
fn main_for(runner: &Runner) -> String {
    let mut main = format!(
        "\n{}\n#include <stdio.h>\n#include <stdlib.h>\n\n\
         int main(int argc, char *argv[]) {{\n    \
         if (argc != {}) {{\n        return 2;\n    }}\n",
        bench::GENERATED_MAIN,
        runner.fields.len() + 1
    );
    for (at, (name, ty)) in runner.fields.iter().enumerate() {
        let arg = format!("argv[{}]", at + 1);
        let line = match ty {
            CType::Signed(c) => format!("{c} {name} = ({c})strtoll({arg}, NULL, 10);"),
            CType::Unsigned(c) => format!("{c} {name} = ({c})strtoull({arg}, NULL, 10);"),
            CType::Float => format!("float {name} = strtof({arg}, NULL);"),
            CType::Double => format!("double {name} = strtod({arg}, NULL);"),
            CType::Bool => format!("int {name} = (int)strtol({arg}, NULL, 10);"),
            CType::String => format!("const char *{name} = {arg};"),
        };
        main += &format!("    {line}\n");
    }
    let args: Vec<String> = runner
        .args
        .iter()
        .map(|arg| match arg {
            Arg::Field(field) => field.clone(),
            Arg::FirstByte(field) => format!("{field}[0]"),
        })
        .collect();
    let call = format!("{}({})", runner.symbol, args.join(", "));
    match &runner.result {
        Some(field) => {
            let ty = runner
                .fields
                .iter()
                .find(|(name, _)| name == field)
                .unwrap()
                .1;
            main += &format!("    {field} = {call};\n");
            main += &format!("    printf(\"{}\\n\", {field});\n", ty.format());
        }
        None => main += &format!("    {call};\n"),
    }
    main + "    return 0;\n}\n"
}

/// Turns a library test vector into an input after checking that it is a
/// valid C reference run with TRACTOR's expected output and result.
fn library_input(
    vector: &serde_json::Value,
    name: &str,
    runner: &Runner,
    references: &eval::References,
) -> Result<eval::Input, &'static str> {
    let known = [
        "lib_state_in",
        "lib_state_out",
        "stdout",
        "stderr",
        "rc",
        "note",
        "has_ub",
        "env",
    ];
    let fields = vector.as_object().ok_or("vector_format")?;
    if fields.keys().any(|key| !known.contains(&key.as_str())) {
        return Err("vector_format");
    }
    if fields.contains_key("has_ub") {
        return Err("marked_undefined_behaviour");
    }
    if vector["env"].as_object().is_some_and(|env| !env.is_empty()) {
        return Err("environment");
    }
    if vector["stdout"]["is_regex"] == true {
        return Err("output_pattern");
    }
    // The program prints only the result field. A string passed as a pointer
    // could still change, which TRACTOR would see and the program would not.
    let (state_in, state_out) = (&vector["lib_state_in"], &vector["lib_state_out"]);
    let unchecked = runner.fields.iter().any(|(field, _)| {
        Some(field) != runner.result.as_ref()
            && state_out
                .get(field)
                .is_some_and(|out| *out != state_in[field])
    });
    if unchecked {
        return Err("unchecked_state_change");
    }
    let args = runner
        .fields
        .iter()
        .map(|(field, ty)| argument(&vector["lib_state_in"][field], *ty))
        .collect::<Option<_>>()
        .ok_or("vector_format")?;
    let input = eval::Input {
        name: name.to_owned(),
        args,
        stdin: Vec::new(),
    };
    let first = references.check(&input)?;
    let mut stdout = first.stdout.as_slice();
    if let Some(field) = &runner.result {
        let ty = runner
            .fields
            .iter()
            .find(|(name, _)| name == field)
            .unwrap()
            .1;
        let printed = stdout.strip_suffix(b"\n").unwrap_or_default();
        let start = printed
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |at| at + 1);
        let value = std::str::from_utf8(&printed[start..]).unwrap_or_default();
        if !same_value(value, &vector["lib_state_out"][field], ty) {
            return Err("c_differs_from_expected");
        }
        stdout = &stdout[..start];
    }
    let expected_exit = vector["rc"].as_i64().unwrap_or(0);
    let expected_stdout = vector["stdout"]["pattern"].as_str();
    if first.exit != Some(expected_exit as i32)
        || expected_stdout.is_some_and(|out| out.as_bytes() != stdout)
    {
        return Err("c_differs_from_expected");
    }
    Ok(input)
}

/// A state value as a command-line argument for the generated `main`.
fn argument(value: &serde_json::Value, ty: CType) -> Option<String> {
    match ty {
        CType::Signed(_) => value.as_i64().map(|n| n.to_string()),
        CType::Unsigned(_) => value.as_u64().map(|n| n.to_string()),
        CType::Float | CType::Double => value.is_number().then(|| value.to_string()),
        CType::Bool => value.as_bool().map(|b| u8::from(b).to_string()),
        CType::String => value.as_str().map(str::to_owned),
    }
}

/// Whether the value the generated `main` printed is TRACTOR's expected one.
/// Floats compare as TRACTOR's harness does: equal within 1e-6, or within the
/// type's machine epsilon relative to the larger value.
fn same_value(printed: &str, expected: &serde_json::Value, ty: CType) -> bool {
    match ty {
        CType::Signed(_) | CType::Unsigned(_) => {
            let expected = expected
                .as_i64()
                .map(i128::from)
                .or(expected.as_u64().map(i128::from));
            expected.is_some() && printed.parse::<i128>().ok() == expected
        }
        CType::Bool => expected
            .as_bool()
            .is_some_and(|b| printed == u8::from(b).to_string()),
        CType::Float | CType::Double => {
            let (Ok(got), Some(want)) = (printed.parse::<f64>(), expected.as_f64()) else {
                return false;
            };
            let (got, want, epsilon) = match ty {
                CType::Float => (
                    f64::from(got as f32),
                    f64::from(want as f32),
                    f64::from(f32::EPSILON),
                ),
                _ => (got, want, f64::EPSILON),
            };
            let difference = (got - want).abs();
            got == want
                || got.is_finite()
                    && want.is_finite()
                    && (difference <= 1e-6 || difference <= got.abs().max(want.abs()) * epsilon)
        }
        CType::String => false,
    }
}

/// Assigns a difficulty group from the C features the program uses: the
/// highest group whose features occur. Signals name the features found.
fn classify(c_file: &Path) -> Result<(u8, Vec<&'static str>), String> {
    let output = Command::new("clang-18")
        .args(["-Xclang", "-ast-dump=json", "-fsyntax-only"])
        .arg(c_file)
        .output()
        .map_err(|err| format!("cannot run clang-18: {err}"))?;
    let ast: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "c_build: no syntax tree".to_owned())?;
    let mut signals = BTreeSet::new();
    for decl in ast["inner"].as_array().into_iter().flatten() {
        let loc = &decl["loc"];
        let own = decl["isImplicit"] != true
            && loc["includedFrom"].is_null()
            && loc["expansionLoc"]["includedFrom"].is_null()
            && (loc["offset"].is_number() || loc["expansionLoc"]["offset"].is_number());
        if own {
            features(decl, false, false, &mut signals);
        }
    }
    let group = |names: &[&str]| names.iter().any(|name| signals.contains(name));
    let group = if group(&["union", "goto", "function_pointer", "void_pointer"]) {
        5
    } else if group(&["heap"]) {
        4
    } else if group(&["struct_pointer", "address_of", "pointer"]) {
        3
    } else if group(&["array", "string", "string_function"]) {
        2
    } else {
        1
    };
    Ok((group, signals.into_iter().collect()))
}

fn features(
    node: &serde_json::Value,
    in_main: bool,
    in_parser: bool,
    signals: &mut BTreeSet<&'static str>,
) {
    let kind = node["kind"].as_str().unwrap_or_default();
    let ty = node["type"]["desugaredQualType"]
        .as_str()
        .or(node["type"]["qualType"].as_str())
        .unwrap_or_default();
    let mut in_main = in_main;
    let mut in_parser = in_parser;
    match kind {
        "FunctionDecl" => in_main = node["name"] == "main",
        "RecordDecl" if node["tagUsed"] == "union" => {
            signals.insert("union");
        }
        "GotoStmt" | "IndirectGotoStmt" => {
            signals.insert("goto");
        }
        "MemberExpr" if node["isArrow"] == true => {
            signals.insert("struct_pointer");
        }
        // Parsing input needs addresses but says nothing about the program.
        "UnaryOperator" if node["opcode"] == "&" && !in_parser => {
            signals.insert("address_of");
        }
        "CallExpr" => in_parser = calls(node, &INPUT_PARSERS),
        "DeclRefExpr" => {
            let name = node["referencedDecl"]["name"].as_str().unwrap_or_default();
            if HEAP.contains(&name) {
                signals.insert("heap");
            }
            if STRING_FUNCTIONS.contains(&name) {
                signals.insert("string_function");
            }
        }
        // main's argc and argv, and strings taken from them, carry the command line.
        "ParmVarDecl" if in_main => {}
        "VarDecl" if in_main && matches!(ty, "char *" | "const char *") => {}
        "VarDecl" | "ParmVarDecl" | "FieldDecl" | "TypedefDecl" => {
            // `int (*)(int)` but not a pointer to an array, `char (*)[8]`.
            if ty.contains("(*") && ty.contains(")(") {
                signals.insert("function_pointer");
            } else if ty.contains("void *") {
                signals.insert("void_pointer");
            } else if ty.contains('[') {
                signals.insert("array");
            } else if matches!(ty, "char *" | "const char *") {
                signals.insert("string");
            } else if ty.contains('*') {
                signals.insert("pointer");
            }
        }
        _ => {}
    }
    for child in node["inner"].as_array().into_iter().flatten() {
        features(child, in_main, in_parser, signals);
    }
}

/// Whether a call expression calls one of `names` directly.
fn calls(call: &serde_json::Value, names: &[&str]) -> bool {
    fn callee(node: &serde_json::Value) -> Option<&str> {
        match node["kind"].as_str()? {
            "DeclRefExpr" => node["referencedDecl"]["name"].as_str(),
            "ImplicitCastExpr" => callee(node["inner"].get(0)?),
            _ => None,
        }
    }
    call["inner"]
        .get(0)
        .and_then(callee)
        .is_some_and(|name| names.contains(&name))
}

/// Assigns families without a split, per group, keeping earlier assignments.
fn assign(
    programs: &[Program],
    inspected: &BTreeSet<String>,
    seed: u64,
    split: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    for family in inspected {
        match split.get(family) {
            Some(set) if set != "dev" => {
                return Err(format!("{family} was inspected but is in {set}"));
            }
            _ => {}
        }
    }
    for group in 1..=5 {
        let in_group: Vec<&Program> = programs.iter().filter(|p| p.group == group).collect();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for program in &in_group {
            if let Some(set) = split.get(&program.family) {
                let (set, _) = SPLITS
                    .iter()
                    .find(|(name, _)| name == set)
                    .ok_or_else(|| format!("unknown split {set} in split.json"))?;
                *counts.entry(set).or_default() += 1;
            } else if inspected.contains(&program.family) {
                split.insert(program.family.clone(), "dev".to_owned());
                *counts.entry("dev").or_default() += 1;
            }
        }
        let mut new: Vec<&Program> = in_group
            .into_iter()
            .filter(|p| !split.contains_key(&p.family))
            .collect();
        new.sort_by_key(|p| (bench::mix(seed ^ bench::fnv(&p.family)), p.family.clone()));
        for program in new {
            // The set furthest below its share after this program is added.
            let (set, _) = SPLITS
                .iter()
                .map(|&(set, share)| (set, counts.get(set).copied().unwrap_or(0) as f64 / share))
                .fold(("", f64::INFINITY), |best, next| {
                    if next.1 < best.1 { next } else { best }
                });
            split.insert(program.family.clone(), set.to_owned());
            *counts.entry(set).or_default() += 1;
        }
    }
    Ok(())
}

fn write_program(program: &Program, dir: &Path) -> Result<(), String> {
    let base = dir.join(&program.file_name);
    std::fs::copy(&program.c_file, base.with_extension("c"))
        .map_err(|err| format!("cannot copy {}: {err}", program.c_file.display()))?;
    // Git stores the text with LF line endings, so a CRLF copy would show as
    // changed after every rerun.
    if let Some(licence) = &program.licence {
        let text = read(licence)?.replace("\r\n", "\n");
        write(&base.with_extension("LICENSE"), &text)?;
    }
    let inputs: Vec<_> = program
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
        "source": format!("TRACTOR {CORPUS_REVISION} {}", program.source),
        "signals": program.signals,
        "inputs": inputs,
    });
    if !program.c_flags.is_empty() {
        json["c_flags"] = serde_json::json!(program.c_flags);
    }
    write(&base.with_extension("json"), &format!("{json:#}\n"))
}

/// Removes earlier benchmark files so a rerun leaves no stale programs.
fn clear(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    for path in eval::entries(dir)? {
        let name = file_name(&path);
        if name.starts_with('g')
            && [".c", ".json", ".LICENSE"]
                .iter()
                .any(|ext| name.ends_with(ext))
        {
            std::fs::remove_file(&path)
                .map_err(|err| format!("cannot remove {}: {err}", path.display()))?;
        }
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("cannot read {}: {err}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group_of(test: &str, c: &str) -> (u8, Vec<&'static str>) {
        let file = std::env::temp_dir().join(format!("intake-{test}-{}.c", std::process::id()));
        std::fs::write(&file, c).unwrap();
        let result = classify(&file);
        std::fs::remove_file(&file).unwrap();
        result.unwrap()
    }

    fn program(family: &str, group: u8) -> Program {
        Program {
            family: family.to_owned(),
            source: String::new(),
            file_name: String::new(),
            c_file: PathBuf::new(),
            licence: None,
            c_flags: Vec::new(),
            group,
            signals: Vec::new(),
            inputs: Vec::new(),
        }
    }

    #[test]
    fn a_licence_is_written_with_lf_line_endings() {
        let dir = std::env::temp_dir().join(format!("intake-licence-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let (c_file, licence) = (dir.join("in.c"), dir.join("LICENSE"));
        std::fs::write(&c_file, "int main(void) { return 0; }\n").unwrap();
        std::fs::write(&licence, "MIT License\r\n\r\nCopyright\r\n").unwrap();
        let program = Program {
            file_name: "g1_x".to_owned(),
            c_file,
            licence: Some(licence),
            ..program("B01/x", 1)
        };
        write_program(&program, &dir).unwrap();
        let written = std::fs::read_to_string(dir.join("g1_x.LICENSE")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(written, "MIT License\n\nCopyright\n");
    }

    #[test]
    fn cmake_arguments_may_span_lines() {
        let text = "project(x) # comment (ignored)\nadd_executable(driver\n  src/main.c)\ntarget_link_libraries(driver m)\n";
        assert_eq!(
            cmake_commands(text),
            [
                ("project".to_owned(), "x".to_owned()),
                (
                    "add_executable".to_owned(),
                    "driver\n  src/main.c".to_owned()
                ),
                ("target_link_libraries".to_owned(), "driver m".to_owned()),
            ]
        );
    }

    #[test]
    fn command_line_parsing_does_not_raise_the_group() {
        let c = "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char *argv[]) {\n    char *end;\n    long n = strtol(argv[1], &end, 10);\n    int x;\n    scanf(\"%d\", &x);\n    printf(\"%ld\\n\", n + x);\n    return 0;\n}\n";
        assert_eq!(group_of("cli", c), (1, Vec::new()));
    }

    #[test]
    fn groups_follow_the_strongest_feature() {
        let pointer = "struct p { int x; };\nvoid set(struct p *p) { p->x = 1; }\nint main(void) { struct p v; set(&v); return v.x; }\n";
        assert_eq!(group_of("pointer", pointer).0, 3);
        let heap =
            "#include <stdlib.h>\nint main(void) { int *p = malloc(4); free(p); return 0; }\n";
        assert_eq!(group_of("heap", heap).0, 4);
        let union =
            "union u { int i; float f; };\nint main(void) { union u v; v.i = 1; return v.i; }\n";
        assert_eq!(group_of("union", union).0, 5);
        let rows = "int sum(int rows[][2]) { return rows[0][1]; }\nint main(void) { int r[1][2] = {{1, 2}}; return sum(r); }\n";
        // Clang types `rows` as `int (*)[2]`, a pointer to an array.
        assert_eq!(group_of("rows", rows), (2, vec!["array"]));
        let callback = "int apply(int (*f)(int), int x) { return f(x); }\nint twice(int x) { return 2 * x; }\nint main(void) { return apply(twice, 1); }\n";
        assert_eq!(group_of("callback", callback).0, 5);
        let array = "int t[3] = {1, 2, 3};\nint main(void) { return t[1]; }\n";
        assert_eq!(group_of("array", array), (2, vec!["array"]));
    }

    #[test]
    fn inspected_families_stay_in_dev_and_assignments_are_kept() {
        let programs: Vec<Program> = (0..12).map(|i| program(&format!("B/f{i}"), 1)).collect();
        let inspected = BTreeSet::from(["B/f0".to_owned(), "B/f1".to_owned()]);
        let mut split = BTreeMap::from([("B/f2".to_owned(), "held-out".to_owned())]);
        assign(&programs, &inspected, 7, &mut split).unwrap();

        assert_eq!(split["B/f0"], "dev");
        assert_eq!(split["B/f1"], "dev");
        assert_eq!(split["B/f2"], "held-out");
        let count = |set: &str| split.values().filter(|s| *s == set).count();
        assert_eq!(
            (count("dev"), count("validation"), count("held-out")),
            (6, 3, 3)
        );

        let mut again = split.clone();
        assign(&programs, &inspected, 99, &mut again).unwrap();
        assert_eq!(again, split, "a new seed must not move assigned families");
    }

    const SCALE_RUNNER: &str = r#"use cando2::*;

harness! {
    state: {
        factor: c_int,
        value: f64,
        returns: f64,
    },
    library: "scale",
    symbol: "scale",
    signature: unsafe extern "C" fn(c_int, f64) -> f64,

    fn run(&mut self) {
        // The value is scaled in place.
        self.returns = unsafe {
            (*SYMBOL)(
                self.factor,
                self.value
            )
        };
    }
}
"#;

    #[test]
    fn a_runner_with_one_plain_call_becomes_a_main() {
        let runner = parse_runner(SCALE_RUNNER, "scale").unwrap();
        assert_eq!(
            main_for(&runner),
            "\n/* Added by the benchmark intake in place of TRACTOR's test runner. */\n\
             #include <stdio.h>\n#include <stdlib.h>\n\n\
             int main(int argc, char *argv[]) {\n    \
             if (argc != 4) {\n        return 2;\n    }\n    \
             int factor = (int)strtoll(argv[1], NULL, 10);\n    \
             double value = strtod(argv[2], NULL);\n    \
             double returns = strtod(argv[3], NULL);\n    \
             returns = scale(factor, value);\n    \
             printf(\"%.17g\\n\", returns);\n    \
             return 0;\n}\n"
        );
    }

    #[test]
    fn a_runner_without_a_symbol_calls_the_function_named_like_its_test_case() {
        let runner = SCALE_RUNNER.replace("    symbol: \"scale\",\n", "");
        assert_eq!(parse_runner(&runner, "other").unwrap().symbol, "other");
    }

    #[test]
    fn strings_pass_as_pointers_or_first_bytes() {
        let runner = "harness! {\n    state: {\n        text: CString,\n        flag: bool,\n    },\n    library: \"l\",\n    symbol: \"show\",\n    signature: unsafe extern \"C\" fn(*const c_char, c_char, bool),\n    fn run(&mut self) {\n        unsafe { (*SYMBOL)(self.text.as_ptr(), self.text.as_bytes()[0] as c_char, self.flag); }\n    }\n}\n";
        let runner = parse_runner(runner, "show").unwrap();
        assert_eq!(runner.result, None);
        assert_eq!(
            runner.args,
            [
                Arg::Field("text".to_owned()),
                Arg::FirstByte("text".to_owned()),
                Arg::Field("flag".to_owned())
            ]
        );
        let main = main_for(&runner);
        assert!(main.contains("    const char *text = argv[1];\n"), "{main}");
        assert!(main.contains("    show(text, text[0], flag);\n"), "{main}");
    }

    #[test]
    fn runners_doing_more_than_one_plain_call_are_not_converted() {
        let variants = [
            ("value: f64,", "value: Vec<u8>,"),
            ("value: f64,", "value: point,"),
            ("self.value\n", "self.value + 1.0\n"),
            ("self.value\n", "self.value as f64 + 1.0\n"),
            ("self.factor,", "self.factor as usize - 1,"),
            ("self.value\n", "self.value as f32 as f64\n"),
            ("self.value\n", "self.value.as_ptr()\n"),
            ("        };\n", "        };\n        self.value = 0.0;\n"),
            // The next quoted text would be the "C" of the signature.
            ("symbol: \"scale\",", "symbol: &*NAME,"),
        ];
        for (from, to) in variants {
            let runner = SCALE_RUNNER.replacen(from, to, 1);
            assert_ne!(runner, SCALE_RUNNER);
            assert_eq!(parse_runner(&runner, "scale"), None, "{to}");
        }
        let cast = SCALE_RUNNER.replacen("self.factor,", "self.factor as std::ffi::c_int,", 1);
        let args = parse_runner(&cast, "scale").unwrap().args;
        assert_eq!(args[0], Arg::Field("factor".to_owned()));
    }

    #[test]
    fn results_match_within_tractors_tolerance() {
        let json = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
        assert!(same_value(
            "0.30000000000000004",
            &json("0.3"),
            CType::Double
        ));
        assert!(same_value("1.0000005", &json("1.0"), CType::Double));
        assert!(!same_value("1.00001", &json("1.0"), CType::Double));
        assert!(same_value(
            "100000002004087734272",
            &json("1e20"),
            CType::Float
        ));
        assert!(!same_value("nan", &json("0.0"), CType::Double));
        assert!(same_value("-3", &json("-3"), CType::Signed("int")));
        assert!(!same_value("3", &json("3.5"), CType::Signed("int")));
        assert!(same_value(
            "18446744073709551615",
            &json("18446744073709551615"),
            CType::Unsigned("unsigned long long")
        ));
        assert!(same_value("1", &json("true"), CType::Bool));
    }

    #[test]
    fn a_library_case_becomes_one_program_with_its_header_inlined() {
        let dir = std::env::temp_dir().join(format!("intake-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let case_dir = dir.join("B01_test/scale_lib");
        let files = [
            (
                "test_case/CMakeLists.txt",
                "cmake_minimum_required(VERSION 3.10)\nproject(Scale)\nadd_library(Scale SHARED src/scale.c)\ntarget_include_directories(Scale PUBLIC include)\ntarget_link_libraries(Scale PUBLIC m)\ninstall(TARGETS Scale DESTINATION lib)\n",
            ),
            (
                "test_case/include/scale.h",
                "#ifndef SCALE_H\r\n#define SCALE_H\r\ndouble scale(int factor, double value);\r\n#endif\r\n",
            ),
            (
                "test_case/src/scale.c",
                "#include <stdio.h>\n#include \"scale.h\"\n\ndouble scale(int factor, double value) {\n    printf(\"scaling\\n\");\n    return factor * value;\n}\n",
            ),
            ("runner/src/main.rs", SCALE_RUNNER),
            (
                "test_vectors/test1.json",
                r#"{"stdout": {"pattern": "scaling\n"}, "lib_state_in": {"factor": 3, "value": 0.1, "returns": 0}, "lib_state_out": {"factor": 3, "value": 0.1, "returns": 0.3}}"#,
            ),
            (
                "test_vectors/test2.json",
                r#"{"lib_state_in": {"factor": -2, "value": 2.5, "returns": 0}, "lib_state_out": {"factor": -2, "value": 2.5, "returns": -5}}"#,
            ),
            (
                "test_vectors/test3.json",
                r#"{"lib_state_in": {"factor": 2, "value": 2.5, "returns": 0}, "lib_state_out": {"factor": 2, "value": 2.5, "returns": 6}}"#,
            ),
            (
                "test_vectors/test4.json",
                r#"{"stdout": {"pattern": "other\n"}, "lib_state_in": {"factor": 1, "value": 1, "returns": 0}, "lib_state_out": {"factor": 1, "value": 1, "returns": 1}}"#,
            ),
            (
                "test_vectors/test5.json",
                r#"{"lib_state_in": {"factor": 1, "value": 1, "returns": 0}, "lib_state_out": {"factor": 2, "value": 1, "returns": 1}}"#,
            ),
        ];
        for (path, text) in files {
            let path = case_dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let case = Case {
            visibility: "Public-Tests".to_owned(),
            battery: "B01_test".to_owned(),
            name: "scale_lib".to_owned(),
            dir: case_dir,
        };
        let scratch = dir.join("scratch");
        std::fs::create_dir(&scratch).unwrap();
        let mut dropped = BTreeMap::new();

        let twin = super::program(
            &case,
            &BTreeSet::from([case.family()]),
            &scratch,
            &mut dropped,
        );
        let converted = super::program(&case, &BTreeSet::new(), &scratch, &mut dropped);
        // A case of the same name in another battery writes its own program.
        let other_dir = dir.join("B02_test/scale_lib");
        for (path, text) in files {
            let path = other_dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text.replace("scaling", "other")).unwrap();
        }
        let other = Case {
            visibility: "Public-Tests".to_owned(),
            battery: "B02_test".to_owned(),
            name: "scale_lib".to_owned(),
            dir: other_dir,
        };
        let _ = super::program(&other, &BTreeSet::new(), &scratch, &mut BTreeMap::new());

        let source = converted.as_ref().ok().map(|p| read(&p.c_file).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(twin.err().as_deref(), Some("library"));
        let program = converted.unwrap();
        assert_eq!(program.file_name, "g1_scale");
        assert_eq!(program.family, "B01_test/scale");
        let names: Vec<&str> = program.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["test1", "test2"]);
        assert_eq!(program.inputs[0].args, ["3", "0.1", "0"]);
        assert_eq!(
            dropped,
            BTreeMap::from([
                ("c_differs_from_expected", 2),
                ("unchecked_state_change", 1)
            ])
        );
        let source = source.unwrap();
        assert!(source.contains("\"scaling\\n\""), "{source}");
        assert!(source.contains("#define SCALE_H\n"));
        assert!(!source.contains("#include \"scale.h\""));
        assert!(!source.contains('\r'));
        assert!(source.ends_with(&main_for(&parse_runner(SCALE_RUNNER, "scale").unwrap())));
    }

    #[test]
    fn inspected_family_outside_dev_is_an_error() {
        let inspected = BTreeSet::from(["B/f0".to_owned()]);
        let mut split = BTreeMap::from([("B/f0".to_owned(), "validation".to_owned())]);
        assert!(assign(&[program("B/f0", 1)], &inspected, 0, &mut split).is_err());
    }
}
