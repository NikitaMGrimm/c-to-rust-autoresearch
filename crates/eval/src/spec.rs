//! Benchmark specs: the inputs a program runs on and its C compiler options.

use std::path::Path;

/// Compiler options a benchmark may request for its C reference.
pub const C_FLAGS: [&str; 1] = ["-fno-strict-aliasing"];

/// Command-line arguments and standard input for one run of a program.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    pub name: String,
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
}

/// How a benchmark program is built and run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub inputs: Vec<Input>,
    /// Extra options for the C reference, from [`C_FLAGS`].
    pub c_flags: Vec<String>,
}

/// Reads the spec of `c_file` from the JSON file next to it, for example
/// `g1_gcd.json` for `g1_gcd.c`. Without that file the program runs once
/// without arguments or input.
pub fn spec(c_file: &Path) -> Result<Spec, String> {
    let path = c_file.with_extension("json");
    let text = match std::fs::read(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Spec {
                inputs: vec![Input::default()],
                c_flags: Vec::new(),
            });
        }
        Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
    };
    let invalid = |why: &str| format!("{}: {why}", path.display());
    let json: serde_json::Value =
        serde_json::from_slice(&text).map_err(|err| invalid(&err.to_string()))?;
    let c_flags = match &json["c_flags"] {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(flags) => flags
            .iter()
            .map(|flag| flag.as_str().filter(|flag| C_FLAGS.contains(flag)))
            .map(|flag| flag.map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or_else(|| invalid(&format!("\"c_flags\" may only contain {C_FLAGS:?}")))?,
        _ => return Err(invalid("\"c_flags\" must be a list")),
    };
    let inputs = parse_inputs(&json["inputs"], invalid)?;
    Ok(Spec { inputs, c_flags })
}

/// Extension of the file next to a benchmark program that holds its fuzzed
/// inputs, in the same form as the spec's `inputs`.
pub const FUZZ_EXTENSION: &str = "fuzz.json";

/// Reads the fuzzed inputs of `c_file`; a program may have none.
pub fn fuzzed_inputs(c_file: &Path) -> Result<Vec<Input>, String> {
    let path = c_file.with_extension(FUZZ_EXTENSION);
    let text = match std::fs::read(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
    };
    let invalid = |why: &str| format!("{}: {why}", path.display());
    let json: serde_json::Value =
        serde_json::from_slice(&text).map_err(|err| invalid(&err.to_string()))?;
    parse_inputs(&json["inputs"], invalid)
}

fn parse_inputs(
    list: &serde_json::Value,
    invalid: impl Fn(&str) -> String,
) -> Result<Vec<Input>, String> {
    let list = list
        .as_array()
        .filter(|list| !list.is_empty())
        .ok_or_else(|| invalid("expected a non-empty \"inputs\" list"))?;
    list.iter()
        .map(|input| {
            let name = input["name"]
                .as_str()
                .ok_or_else(|| invalid("every input needs a \"name\""))?;
            let args = match &input["args"] {
                serde_json::Value::Null => Vec::new(),
                serde_json::Value::Array(args) => args
                    .iter()
                    .map(|arg| arg.as_str().map(str::to_owned))
                    .collect::<Option<_>>()
                    .ok_or_else(|| invalid("\"args\" must be strings"))?,
                _ => return Err(invalid("\"args\" must be a list")),
            };
            let stdin = match &input["stdin"] {
                serde_json::Value::Null => Vec::new(),
                serde_json::Value::String(stdin) => stdin.as_bytes().to_vec(),
                _ => return Err(invalid("\"stdin\" must be a string")),
            };
            Ok(Input {
                name: name.to_owned(),
                args,
                stdin,
            })
        })
        .collect()
}
