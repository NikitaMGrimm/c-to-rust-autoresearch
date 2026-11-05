//! Shared parts of the benchmark tools: seeded random numbers and the numbers
//! in program inputs.

use std::path::{Path, PathBuf};

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
