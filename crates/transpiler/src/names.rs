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
    "scanned",
    "parse_int",
    "peek_byte",
    "read_byte",
    "skip_space",
    "scan_int",
    "read_line",
    "read_line_within",
    "parse_uint",
    "copy_text",
    "store_text",
    "store_text_cut",
    "store_text_within",
    "parse_int_prefix",
    "copy_string",
    "append_string",
    "append_at_most",
    "parse_float",
    "format_e",
    "format_g",
    "until_nul",
    "text_of",
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
        let records = program
            .records
            .iter()
            .map(|record| unique(camel(&record.name), &record.name, &mut taken))
            .collect();
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
                    let mut name = plain(&field.name);
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

/// The name, or the fallback with underscores added until it is free.
fn unique(name: String, fallback: &str, taken: &mut HashSet<String>) -> String {
    let mut name = name;
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
