//! Shared parts of the benchmark tools: seeded random numbers, the numbers in
//! program inputs and a program's difficulty group.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use eval::Input;

/// The benchmark sets, each with its directory: development in this
/// repository, validation and held-out in the private directory.
pub fn sets(private: &Path) -> [(&'static str, PathBuf); 3] {
    [
        ("dev", eval::dev_dir()),
        ("validation", private.join("validation")),
        ("held-out", private.join("held-out")),
    ]
}

/// The comment that starts the `main` intake writes for a library test case.
/// The rest of the file is that `main`, which is not TRACTOR's code.
pub const GENERATED_MAIN: &str =
    "/* Added by the benchmark intake in place of TRACTOR's test runner. */";

/// The secret seed in the private directory, from which every assignment and
/// variant is derived.
pub fn seed(private: &Path) -> Result<u64, String> {
    let path = private.join("seed");
    let seed = std::fs::read_to_string(&path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    u64::from_str_radix(seed.trim(), 16).map_err(|_| "seed must be hexadecimal".to_owned())
}

/// SplitMix64 finaliser: spreads a seed into an evenly distributed value.
pub fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Deterministic random numbers from a seed, so generated benchmarks can be
/// regenerated exactly.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        mix(self.0)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// FNV-1a hash, to derive seeds from names.
pub fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

/// Functions whose behaviour is undefined when a number they read does not
/// fit its type. The sanitizers miss this, so generated inputs for programs
/// that call them keep their numbers within `int`.
const UNCHECKED_PARSERS: [&str; 6] = ["atoi", "atol", "atoll", "scanf", "sscanf", "fscanf"];

/// Whether the C `source` calls one of [`UNCHECKED_PARSERS`]. A mention in a
/// comment also counts, which only makes generated inputs smaller.
pub fn parses_numbers_unchecked(source: &str) -> bool {
    let bytes = source.as_bytes();
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut i = 0;
    while i < bytes.len() {
        if !word(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && word(bytes[i]) {
            i += 1;
        }
        let called = source[i..].trim_start().starts_with('(');
        if called && UNCHECKED_PARSERS.contains(&&source[start..i]) {
            return true;
        }
    }
    false
}

/// Byte ranges of the decimal numbers in `text`, with a minus sign that does
/// not follow a word or another number.
pub fn number_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let mut start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let glued = |at: usize| bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_';
        if start > 0 && bytes[start - 1] == b'-' && (start == 1 || !glued(start - 2)) {
            start -= 1;
        }
        found.push((start, i));
    }
    found
}

/// Whether every number in the arguments and standard input fits in `int`.
pub fn numbers_fit_int(input: &Input) -> bool {
    let stdin = String::from_utf8_lossy(&input.stdin);
    input
        .args
        .iter()
        .map(String::as_str)
        .chain([stdin.as_ref()])
        .all(|text| {
            number_spans(text)
                .iter()
                .all(|&(start, end)| text[start..end].parse::<i32>().is_ok())
        })
}

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

/// Assigns a difficulty group from the C features the program uses: the
/// highest group whose features occur. Signals name the features found.
pub fn classify(c_file: &Path) -> Result<(u8, Vec<&'static str>), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchecked_number_parsers_are_found_only_when_called() {
        assert!(parses_numbers_unchecked("int n = atoi (argv[1]);"));
        assert!(parses_numbers_unchecked(
            "if (sscanf(line, \"%d\", &n) != 1)"
        ));
        assert!(!parses_numbers_unchecked(
            "long n = strtol(argv[1], 0, 10);"
        ));
        assert!(!parses_numbers_unchecked(
            "int my_atoi(const char *s); int atoi_count;"
        ));
    }

    #[test]
    fn numbers_beyond_int_are_found_in_arguments_and_input() {
        let input = |args: &[&str], stdin: &str| Input {
            name: String::new(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            stdin: stdin.as_bytes().to_vec(),
        };
        assert!(numbers_fit_int(&input(
            &["2147483647", "x-5"],
            "-2147483648\n"
        )));
        assert!(!numbers_fit_int(&input(&["2147483648"], "")));
        assert!(!numbers_fit_int(&input(&[], "1 -2147483649\n")));
    }
}
