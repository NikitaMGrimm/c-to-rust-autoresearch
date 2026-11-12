//! The Rust names of the program's variables, functions and records. A name
//! stays as in C unless Rust reserves it or its convention differs.

use crate::c::{FuncId, Program, RecordId, VarId, VarKind};
use crate::facts::Facts;
use std::collections::{HashMap, HashSet};

/// Rust's keywords that C allows as names, and the names that the
/// translation itself uses.
const RESERVED: [&str; 45] = [
    "as", "async", "await", "become", "box", "crate", "dyn", "fn", "gen", "impl", "in", "let",
    "loop", "match", "mod", "move", "mut", "pub", "ref", "self", "Self", "super", "trait", "try",
    "type", "unsafe", "use", "where", "yield", "abstract", "final", "macro", "override", "priv",
    "true", "false", "virtual", "args", "globals", "drop", "main", "Some", "None", "Globals",
    "before",
];

pub struct Names {
    vars: Vec<String>,
    records: Vec<String>,
    functions: Vec<String>,
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
                VarKind::Static(function) => {
                    format!("{}_{}", program.functions[function].name, var.name)
                }
                _ => format!("{}_", var.name),
            };
            vars[global] = unique(name, &fallback, &mut taken);
        }
        // A local may hide another local of the same C name, as in C, but
        // not a function or a constant that its scope uses, and two C names
        // never share a Rust name: a name that must change gets underscores
        // until no local has it.
        let locals = || program.vars.iter().filter(|var| !var.kind.is_global());
        let free = |name: &str| !RESERVED.contains(&name) && !taken.contains(name);
        let mut given: HashSet<String> = locals()
            .filter(|var| free(&var.name))
            .map(|var| var.name.clone())
            .collect();
        let mut changed: HashMap<&str, String> = HashMap::new();
        for var in locals().filter(|var| !free(&var.name)) {
            changed.entry(&var.name).or_insert_with(|| {
                let mut name = format!("{}_", var.name);
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
            functions,
        }
    }

    pub fn var(&self, var: VarId) -> &str {
        &self.vars[var]
    }

    pub fn record(&self, record: RecordId) -> &str {
        &self.records[record]
    }

    pub fn function(&self, function: FuncId) -> &str {
        &self.functions[function]
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

/// A C name as a Rust name for a value or a field.
pub fn plain(name: &str) -> String {
    if RESERVED.contains(&name) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
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
