//! Converts the standalone programs of DARPA's TRACTOR test corpus into
//! benchmarks and splits them by family into development, validation and
//! held-out sets. Only names, groups and counts are printed, so running it
//! does not show validation or held-out programs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const CORPUS_URL: &str = "https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus";
const CORPUS_REVISION: &str = "37960ee08a7c644ee66e29855eca33903f702e67";
const USAGE: &str = "usage: intake CORPUS_DIR PRIVATE_DIR\n\
     CORPUS_DIR is fetched when missing; keep it outside this repository.\n\
     PRIVATE_DIR is a checkout of the private holdout repository with a `seed` file.";
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
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let seed = read(&private.join("seed"))?;
    let seed = u64::from_str_radix(seed.trim(), 16).map_err(|_| "seed must be hexadecimal")?;
    let inspected: BTreeSet<String> = read(&repo.join("bench/inspected.txt"))?
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();

    let scratch = std::env::temp_dir().join(format!("c2r-intake-{}", std::process::id()));
    std::fs::create_dir(&scratch).map_err(|err| format!("cannot create scratch: {err}"))?;
    let mut programs = Vec::new();
    let mut excluded: BTreeMap<String, String> = BTreeMap::new();
    let mut dropped: BTreeMap<&'static str, usize> = BTreeMap::new();
    for case in cases(corpus)? {
        match program(&case, &scratch, &mut dropped) {
            Ok(program) => programs.push(program),
            Err(reason) => {
                excluded.insert(case.source(), reason);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);

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

    let dev = repo.join("bench/dev");
    let outputs = [
        ("dev", dev.as_path()),
        ("validation", &private.join("validation")),
        ("held-out", &private.join("held-out")),
    ];
    for (_, dir) in outputs {
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
        for battery in sorted_dir(&corpus.join(visibility))? {
            let battery_name = file_name(&battery);
            if !(battery_name.starts_with("B01_") || battery_name.starts_with("B02_")) {
                continue;
            }
            for dir in sorted_dir(&battery)? {
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

/// Converts an executable test case, or says why it cannot be used.
fn program(
    case: &Case,
    scratch: &Path,
    dropped: &mut BTreeMap<&'static str, usize>,
) -> Result<Program, String> {
    if case.name.ends_with("_lib") {
        return Err("library".to_owned());
    }
    let test_case = case.dir.join("test_case");
    let sources = sorted_dir(&test_case.join("src"))?;
    let [c_file] = sources.as_slice() else {
        return Err("multiple_files".to_owned());
    };
    if test_case.join("include").exists() || !file_name(c_file).ends_with(".c") {
        return Err("multiple_files".to_owned());
    }
    if test_case.join("configurations.json").exists() {
        return Err("build_settings".to_owned());
    }
    for (command, args) in cmake_commands(&read(&test_case.join("CMakeLists.txt"))?) {
        let allowed = matches!(
            command.as_str(),
            "cmake_minimum_required" | "project" | "add_executable"
        )
            // The evaluator always links the maths library.
            || command == "target_link_libraries" && args.split_whitespace().nth(1) == Some("m");
        if !allowed {
            return Err(format!("build_settings: {command}"));
        }
    }

    let work = scratch.join(&case.name);
    std::fs::create_dir(&work).map_err(|err| format!("cannot create scratch: {err}"))?;
    let c_program = work.join("c_program");
    if eval::build_c(c_file, &c_program).exit != Some(0) {
        return Err("c_build".to_owned());
    }
    let mut inputs = Vec::new();
    for vector in sorted_dir(&case.dir.join("test_vectors"))? {
        if !file_name(&vector).ends_with(".json") {
            continue;
        }
        let json: serde_json::Value = serde_json::from_str(&read(&vector)?)
            .map_err(|err| format!("vector_format: {}: {err}", vector.display()))?;
        match input(
            &json,
            file_name(&vector).trim_end_matches(".json"),
            &c_program,
        ) {
            Ok(input) => inputs.push(input),
            Err(reason) => *dropped.entry(reason).or_default() += 1,
        }
    }
    if inputs.is_empty() {
        return Err("no_valid_inputs".to_owned());
    }
    let (group, signals) = classify(c_file)?;
    let licence = test_case.join("LICENSE");
    Ok(Program {
        family: case.family(),
        source: case.source(),
        file_name: format!("g{group}_{}", case.name),
        c_file: c_file.clone(),
        licence: licence.exists().then_some(licence),
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

/// Turns a test vector into an input after checking that the sanitized C
/// program gives TRACTOR's expected result, twice.
fn input(
    vector: &serde_json::Value,
    name: &str,
    c_program: &Path,
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
    let input = eval::Input {
        name: name.to_owned(),
        args,
        stdin: vector["stdin"]
            .as_str()
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
    };
    let first = eval::run_program(c_program, &input);
    if first.timed_out() {
        return Err("c_timeout");
    }
    if first.sanitizer_error() {
        return Err("c_sanitizer_error");
    }
    if eval::run_program(c_program, &input) != first {
        return Err("c_nondeterministic");
    }
    let expected_exit = vector["rc"].as_i64().unwrap_or(0);
    let expected_stdout = stdout["pattern"].as_str();
    if first.exit != Some(expected_exit as i32)
        || expected_stdout.is_some_and(|out| out.as_bytes() != first.stdout)
    {
        return Err("c_differs_from_expected");
    }
    Ok(input)
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
            if ty.contains("(*") {
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
        new.sort_by_key(|p| (mix(seed ^ fnv(&p.family)), p.family.clone()));
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

/// SplitMix64 finaliser: spreads a seed into an evenly distributed order key.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn write_program(program: &Program, dir: &Path) -> Result<(), String> {
    let base = dir.join(&program.file_name);
    std::fs::copy(&program.c_file, base.with_extension("c"))
        .map_err(|err| format!("cannot copy {}: {err}", program.c_file.display()))?;
    if let Some(licence) = &program.licence {
        std::fs::copy(licence, base.with_extension("LICENSE"))
            .map_err(|err| format!("cannot copy {}: {err}", licence.display()))?;
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
    let json = serde_json::json!({
        "source": format!("TRACTOR {CORPUS_REVISION} {}", program.source),
        "signals": program.signals,
        "inputs": inputs,
    });
    write(&base.with_extension("json"), &format!("{json:#}\n"))
}

/// Removes earlier benchmark files so a rerun leaves no stale programs.
fn clear(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    for path in sorted_dir(dir)? {
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

fn sorted_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = std::fs::read_dir(dir)
        .and_then(|entries| {
            entries
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()
        })
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    paths.sort();
    Ok(paths)
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
            group,
            signals: Vec::new(),
            inputs: Vec::new(),
        }
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

    #[test]
    fn inspected_family_outside_dev_is_an_error() {
        let inspected = BTreeSet::from(["B/f0".to_owned()]);
        let mut split = BTreeMap::from([("B/f0".to_owned(), "validation".to_owned())]);
        assert!(assign(&[program("B/f0", 1)], &inspected, 0, &mut split).is_err());
    }
}
