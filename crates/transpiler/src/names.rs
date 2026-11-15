//! The Rust names of the program's variables, functions and records. A name
//! stays as in C unless Rust reserves it or its convention differs.

use crate::c::{FuncId, Program, RecordId, VarId, VarKind};
use crate::facts::Facts;
use std::collections::{HashMap, HashSet};

/// Rust's keywords that C allows as names, and the names that the
/// translation itself uses.
const RESERVED: &[&str] = &[
    "as",
    "async",
    "await",
    "become",
    "box",
    "crate",
    "dyn",
    "fn",
    "gen",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "self",
    "Self",
    "super",
    "trait",
    "try",
    "type",
    "unsafe",
    "use",
    "where",
    "yield",
    "abstract",
    "final",
    "macro",
    "override",
    "priv",
    "true",
    "false",
    "virtual",
    "_",
    "args",
    "globals",
    "drop",
    "main",
    "Some",
    "None",
    "Globals",
    "before",
    "assert_holds",
    "grown_len",
    "scanned",
    "random",
    "Random",
    "entry",
    "lower",
    "upper",
    "parse_int",
    "peek_byte",
    "read_byte",
    "skip_space",
    "scan_int",
    "scan_int_or_end",
    "ScanError",
    "Float",
    "Result",
    "Ok",
    "Err",
    "read_line",
    "read_line_within",
    "read_items",
    "read_items_within",
    "parse_uint",
    "copy_text",
    "store_text",
    "store_text_cut",
    "store_text_within",
    "parse_int_prefix",
    "parse_uint_prefix",
    "parse_float_prefix",
    "parse_int_checked",
    "parse_uint_checked",
    "parse_float_checked",
    "hex_float",
    "exact_decimal",
    "copy_string",
    "append_string",
    "append_at_most",
    "parse_float",
    "format_e",
    "format_g",
    "until_nul",
    "text_of",
    "text_within",
    "first_bytes",
];

pub struct Names {
    vars: Vec<String>,
    records: Vec<String>,
    /// The names of each record's fields.
    fields: Vec<Vec<String>>,
    functions: Vec<String>,
    enumerators: Vec<String>,
}

impl Names {
    pub fn new(program: &Program, facts: &Facts) -> Names {
        let mut taken: HashSet<String> = HashSet::new();
        // A record without a tag is named after the variable or field that
        // has it, and a field's after the record that holds it too where
        // that name is taken.
        let mut records: Vec<String> = Vec::new();
        for record in &program.records {
            let fallback = match record.within {
                Some(outer) => format!("{}{}", records[outer], camel(&record.name)),
                None if record.untagged => camel(&record.name),
                None => record.name.clone(),
            };
            let name = unique(camel(&record.name), &fallback, &mut taken);
            records.push(name);
        }
        let functions = program
            .functions
            .iter()
            .map(|function| match function.name.as_str() {
                "main" => "main".to_owned(),
                name => unique(plain(name), name, &mut taken),
            })
            .collect();
        // Globals share one scope with each other and with the functions.
        let mut vars = vec![String::new(); program.vars.len()];
        for &global in &program.globals {
            let var = &program.vars[global];
            let name = match var.kind {
                // A global that never changes becomes a constant.
                _ if !facts.mutated(global) => plain(&var.name).to_uppercase(),
                _ => plain(&var.name),
            };
            let fallback = match var.kind {
                VarKind::Static(function) => snake(&format!(
                    "{}_{}",
                    program.functions[function].name, var.name
                )),
                _ => format!("{}_", snake(&var.name)),
            };
            vars[global] = unique(name, &fallback, &mut taken);
        }
        // An enumerator is a constant beside the globals, wherever C
        // declares it: two functions may each have one of the same name.
        let enumerators = program
            .enumerators
            .iter()
            .map(|enumerator| {
                let name = plain(&enumerator.name).to_uppercase();
                unique(name.clone(), &name, &mut taken)
            })
            .collect();
        // A field keeps its name unless Rust's snake case changes it, and
        // then it gets underscores until no field of its record has it.
        let fields = program
            .records
            .iter()
            .map(|record| {
                let keeps = |name: &str| !RESERVED.contains(&name) && snake(name) == name;
                let mut given: HashSet<&str> = HashSet::new();
                given.extend(record.fields.iter().map(|field| field.name.as_str()));
                let mut changed: HashSet<String> = HashSet::new();
                let mut names = Vec::new();
                for field in &record.fields {
                    if !field.name.is_empty() && keeps(&field.name) {
                        names.push(field.name.clone());
                        continue;
                    }
                    // A member without a name that is a record, as C11
                    // allows, holds that record's fields.
                    let mut name = match field.name.is_empty() && field.bits.is_none() {
                        true => "anonymous".to_owned(),
                        false => plain(&field.name),
                    };
                    while name.is_empty()
                        || RESERVED.contains(&name.as_str())
                        || given.contains(name.as_str())
                        || changed.contains(&name)
                    {
                        name.push('_');
                    }
                    changed.insert(name.clone());
                    names.push(name);
                }
                names
            })
            .collect();
        // A local may hide another local of the same C name, as in C, but
        // not a function or a constant that its scope uses, and two C names
        // never share a Rust name: a name that must change, for Rust's
        // snake case or because it is taken, gets underscores until no
        // local has it.
        let locals = || program.vars.iter().filter(|var| !var.kind.is_global());
        let free = |name: &str| !RESERVED.contains(&name) && !taken.contains(name);
        let keeps = |name: &str| free(name) && snake(name) == name;
        let mut given: HashSet<String> = locals()
            .filter(|var| keeps(&var.name))
            .map(|var| var.name.clone())
            .collect();
        let mut changed: HashMap<&str, String> = HashMap::new();
        for var in locals().filter(|var| !keeps(&var.name)) {
            changed.entry(&var.name).or_insert_with(|| {
                let mut name = snake(&var.name);
                while !free(&name) || given.contains(&name) {
                    name.push('_');
                }
                given.insert(name.clone());
                name
            });
        }
        for (id, var) in program.vars.iter().enumerate() {
            if !var.kind.is_global() {
                vars[id] = match changed.get(var.name.as_str()) {
                    Some(name) => name.clone(),
                    None => var.name.clone(),
                };
            }
        }
        Names {
            vars,
            records,
            fields,
            functions,
            enumerators,
        }
    }

    pub fn var(&self, var: VarId) -> &str {
        &self.vars[var]
    }

    pub fn record(&self, record: RecordId) -> &str {
        &self.records[record]
    }

    pub fn field(&self, record: RecordId, field: usize) -> &str {
        &self.fields[record][field]
    }

    pub fn function(&self, function: FuncId) -> &str {
        &self.functions[function]
    }

    pub fn enumerator(&self, enumerator: usize) -> &str {
        &self.enumerators[enumerator]
    }
}

/// The types and values of Rust's prelude that a translation names: a
/// record with one of their names would hide it.
const PRELUDE: &[&str] = &[
    "Box",
    "Vec",
    "String",
    "Option",
    "Some",
    "None",
    "Result",
    "Ok",
    "Err",
    "Default",
    "Clone",
    "Copy",
    "Drop",
    "Iterator",
    "From",
    "Into",
    "ToString",
    "Eq",
    "Ord",
    "PartialEq",
    "PartialOrd",
];

/// The name, or the fallback with underscores added until it is free.
fn unique(name: String, fallback: &str, taken: &mut HashSet<String>) -> String {
    let mut name = name;
    if PRELUDE.contains(&name.as_str()) {
        name.push('_');
    }
    if taken.contains(&name) {
        name = fallback.to_owned();
    }
    while name.is_empty() || taken.contains(&name) || RESERVED.contains(&name.as_str()) {
        name.push('_');
    }
    taken.insert(name.clone());
    name
}

/// A C name as a Rust name for a value, a function or a field: in snake
/// case, and not a name that Rust or the translation reserves.
fn plain(name: &str) -> String {
    let name = snake(name);
    if RESERVED.contains(&name.as_str()) {
        format!("{name}_")
    } else {
        name
    }
}

/// `printHexLine` as `print_hex_line`: the name as the compiler's lint for
/// snake case writes it, with the underscores it starts and ends with.
/// The Rust name of a C label, for the block that a `goto` leaves. It is
/// no keyword and none of the labels that a `switch` or a loop gets.
pub fn label(name: &str) -> String {
    let name = snake(name);
    let looped = name
        .strip_prefix("outer")
        .is_some_and(|rest| rest.is_empty() || rest.strip_prefix('_').is_some_and(is_number));
    let looped = looped
        || name
            .strip_prefix("next")
            .is_some_and(|rest| rest.is_empty() || rest.strip_prefix('_').is_some_and(is_number));
    match RESERVED.contains(&name.as_str()) || name.starts_with("switch") || looped {
        true => format!("{name}_label"),
        false => name,
    }
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// The label of a C loop at this depth, which a `break` or `continue`
/// names from inside a block entered at a label: one inside another needs
/// a label of its own.
pub fn loop_label(depth: usize) -> String {
    match depth {
        0 | 1 => "outer".to_owned(),
        depth => format!("outer_{depth}"),
    }
}

/// The label of the body of a loop that has something to do after it,
/// which a `continue` leaves.
pub fn step_label(depth: usize) -> String {
    match depth {
        0 | 1 => "next".to_owned(),
        depth => format!("next_{depth}"),
    }
}

/// The name of the variable that says at which part a block entered at a
/// label begins, and of the loop that enters it: `entry` is reserved, so
/// no C name and no C label becomes it.
pub const ENTRY: &str = "entry";

fn snake(name: &str) -> String {
    let inner = name.trim_matches('_');
    let (before, after) = name.split_once(inner).unwrap_or((name, ""));
    let mut words = Vec::new();
    for part in inner.split('_').filter(|part| !part.is_empty()) {
        let mut word = String::new();
        let mut last_upper = false;
        for c in part.chars() {
            if !word.is_empty() && c.is_uppercase() && !last_upper {
                words.push(std::mem::take(&mut word));
            }
            last_upper = c.is_uppercase();
            word.extend(c.to_lowercase());
        }
        words.push(word);
    }
    format!("{before}{}{after}", words.join("_"))
}

/// `house_t` as `HouseT`: the name of a type.
fn camel(name: &str) -> String {
    let mut camel = String::new();
    for part in name.split('_').filter(|part| !part.is_empty()) {
        let mut chars = part.chars();
        camel.extend(chars.next().map(|first| first.to_ascii_uppercase()));
        camel.push_str(chars.as_str());
    }
    camel
}
