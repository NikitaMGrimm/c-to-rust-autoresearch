//! Turns Clang's syntax tree into Rust. Every Rust expression has the Rust type
//! of its C expression, since Clang spells out each implicit conversion. A
//! construct without a rule becomes a `compile_error!`, so the translation
//! fails to build where it would otherwise behave differently.

use crate::control::{self, CLoop, Label, Scope};
use crate::stdio;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub(crate) fn kind(node: &Value) -> &str {
    node["kind"].as_str().unwrap_or("")
}

pub(crate) fn inner(node: &Value) -> &[Value] {
    node["inner"].as_array().map_or(&[], Vec::as_slice)
}

/// The C type of a node, with typedefs resolved and qualifiers dropped.
pub(crate) fn c_type(node: &Value) -> &str {
    qual_type(&node["type"])
}

/// A C type as Clang gives it, with typedefs resolved and qualifiers dropped,
/// including those of a pointer itself, as in `int *restrict`.
fn qual_type(ty: &Value) -> &str {
    let ty = ty["desugaredQualType"]
        .as_str()
        .or(ty["qualType"].as_str())
        .unwrap_or("");
    let mut ty = ty
        .trim_start_matches("const ")
        .trim_start_matches("volatile ");
    while let Some(rest) = ["restrict", "const", "volatile"]
        .iter()
        .find_map(|qualifier| {
            let rest = ty.strip_suffix(qualifier)?.trim_end();
            rest.ends_with('*').then_some(rest)
        })
    {
        ty = rest;
    }
    ty
}

/// The Rust type of a C integer type on x86-64 Linux, where `char` is signed.
pub(crate) fn rust_type(c: &str) -> Option<&'static str> {
    Some(match c {
        "char" | "signed char" => "i8",
        "unsigned char" => "u8",
        "short" => "i16",
        "unsigned short" => "u16",
        "int" => "i32",
        "unsigned int" => "u32",
        "long" | "long long" => "i64",
        "unsigned long" | "unsigned long long" => "u64",
        _ => return None,
    })
}

/// The Rust type of a C floating type: `float` and `double` are IEEE 754
/// single and double precision on x86-64 Linux, as are `f32` and `f64`.
pub(crate) fn float_type(c: &str) -> Option<&'static str> {
    match c {
        "float" => Some("f32"),
        "double" => Some("f64"),
        _ => None,
    }
}

/// The Rust type of a C integer or floating type.
pub(crate) fn number_type(c: &str) -> Option<&'static str> {
    rust_type(c).or(float_type(c))
}

/// Whether `c` is C's boolean type, which `<stdbool.h>` spells `bool`.
pub(crate) fn is_bool(c: &str) -> bool {
    matches!(c, "bool" | "_Bool")
}

/// The Rust type of a C type other than a struct that the translation can
/// hold in a variable: an integer, a `float` or `double`, a `bool`, a `char *`
/// into a string literal, a program argument or a local `char` array, a
/// `char **` into `main`'s `argv`, a pointer to an integer variable or array
/// element, or the result of `div`. A pointer is `None` for null. A `char *`
/// is otherwise the bytes from where it points through the end of its array,
/// and a pointer to an integer is the cells from the one it points to through
/// the end of its array, a variable being an array of one. A `char **` is the
/// pointers from where it points through the null that ends `argv`.
fn builtin_type(c: &str) -> Option<String> {
    match c {
        c if is_bool(c) => Some("bool".to_string()),
        "char *" => Some("Option<&[u8]>".to_string()),
        "char **" => Some("&'static [Option<&'static [u8]>]".to_string()),
        c => match c.strip_suffix(" *") {
            Some(pointee) => cell_type(pointee).map(|ty| format!("Option<&[{CELL}<{ty}>]>")),
            None => number_type(c)
                .map(String::from)
                .or_else(|| quotient_type(c).map(|ty| format!("({ty}, {ty})"))),
        },
    }
}

/// Rust's cell, which holds every integer that a C pointer can point to, so
/// that pointers to the same object can all read and write it, as in C.
pub(crate) const CELL: &str = "std::cell::Cell";

/// The Rust type of an integer that the translation holds in a cell when the
/// program points to it: any but `char`, whose pointers R4 holds as string
/// bytes.
fn cell_type(c: &str) -> Option<&'static str> {
    rust_type(c).filter(|_| c != "char")
}

/// The element type and the length of the C array type `c`: digits for a
/// fixed length, or the expression of a variable length as Clang prints it.
/// The element of an array of arrays is an array.
fn array_type(c: &str) -> Option<(&str, &str)> {
    let (element, length) = c.strip_suffix(']')?.rsplit_once('[')?;
    Some((element.trim_end(), length))
}

/// Whether the length of an array, as `array_type` gives it, is fixed.
fn fixed_length(length: &str) -> bool {
    !length.is_empty() && length.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `c` is an array of `char`, which the translation holds as bytes.
fn char_array(c: &str) -> bool {
    array_type(c).is_some_and(|(element, _)| element == "char")
}

/// The Rust type of the members of `div_t`, `ldiv_t` or `lldiv_t`, which
/// the translation holds as the pair `(quot, rem)`.
fn quotient_type(c: &str) -> Option<&'static str> {
    match c {
        "div_t" => Some("i32"),
        "ldiv_t" | "lldiv_t" => Some("i64"),
        _ => None,
    }
}

/// Whether `expr` is a null pointer constant converted to a pointer, such as
/// `NULL`. Clang converts only null pointer constants with `NullToPointer`.
pub(crate) fn null_pointer(expr: &Value) -> bool {
    match (kind(expr), expr["castKind"].as_str()) {
        ("ImplicitCastExpr" | "CStyleCastExpr", Some("NullToPointer")) => true,
        ("ParenExpr", _) | ("ImplicitCastExpr" | "CStyleCastExpr", Some("NoOp" | "BitCast")) => {
            inner(expr).first().is_some_and(null_pointer)
        }
        _ => false,
    }
}

pub(crate) fn unsupported(what: &str) -> String {
    format!("compile_error!({:?})", format!("unsupported C: {what}"))
}

/// Identifiers that Rust reserves, as a raw identifier where Rust allows one.
/// The others, and `main`, which Rust's own `main` takes, get one more `_`,
/// as does any of them followed by underscores, so no two C names meet.
pub(crate) fn ident(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "as", "async", "await", "box", "break", "const", "continue", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "macro", "match",
        "mod", "move", "mut", "priv", "pub", "ref", "return", "static", "struct", "trait", "true",
        "try", "type", "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
        "abstract", "become", "do", "final", "override",
    ];
    match name {
        // `_` trims to the empty name.
        _ if matches!(
            name.trim_end_matches('_'),
            "self" | "Self" | "super" | "crate" | "" | "main"
        ) =>
        {
            format!("{name}_")
        }
        _ if KEYWORDS.contains(&name) => format!("r#{name}"),
        _ => name.to_string(),
    }
}

/// C11 7.1.3 reserves these names for the implementation, so a definition
/// with one comes from a system header, such as glibc's `__bswap_16`.
fn reserved(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next() == Some('_')
        && chars
            .next()
            .is_some_and(|c| c == '_' || c.is_ascii_uppercase())
}

/// A C struct, which the translation holds as a Rust struct whose members
/// of a scalar type are cells, so that pointers to it can all read and write
/// them, as R10's cells let pointers to integers do.
struct Record {
    /// Its Rust name.
    name: String,
    /// Its members: their names, C types and widths if they are bit-fields.
    members: Vec<(String, String, Option<u32>)>,
    /// Whether the translation names it, so it needs a definition.
    used: std::cell::Cell<bool>,
    /// Whether the translation assigns it whole, so it needs a `set`.
    assigned: std::cell::Cell<bool>,
}

/// Names that the translation uses for Rust's own types, values and paths,
/// which a struct may not take.
const RUST_NAMES: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32",
    "f64", "bool", "char", "str", "Option", "Some", "None", "Vec", "String", "Box", "Result", "Ok",
    "Err", "Default", "Clone", "std", "core", "crt",
];

pub(crate) struct Translator {
    /// Names of the functions the program defines.
    functions: HashSet<String>,
    /// The structs the program defines at file scope.
    records: Vec<Record>,
    /// The record of each C type that names a struct, as `struct tag` or by
    /// a typedef.
    record_types: HashMap<String, usize>,
    /// The width of each bit-field member, by the id of its declaration.
    bitfields: HashMap<String, u32>,
    /// Ids of the variables that are assigned or whose address is taken.
    mutated: HashSet<String>,
    /// Ids of the integer variables held in a cell, as the program points
    /// to them.
    cells: HashSet<String>,
    /// The Rust statics of the static local variables, by id.
    statics: HashMap<String, String>,
    /// The Rust functions that give the global variables the translation
    /// holds, by the id of each of their declarations, and whether each is
    /// an array.
    globals: HashMap<String, (String, bool)>,
    /// The parts of the `crt` runtime the translation calls, in first use.
    runtime: Vec<&'static str>,
    /// Rust loops and labelled blocks around the current statement.
    pub(crate) scopes: Vec<Scope>,
    /// The C loops around the current statement, innermost last.
    pub(crate) loops: Vec<CLoop>,
    /// The labels that a `goto` in the current statement can reach, by id.
    pub(crate) labels: HashMap<String, Label>,
    /// Numbers the labels and variables the translation introduces.
    pub(crate) fresh: usize,
}

pub(crate) fn program(ast: &Value) -> String {
    let definitions: Vec<&Value> = inner(ast)
        .iter()
        .filter(|decl| {
            kind(decl) == "FunctionDecl"
                && inner(decl).iter().any(|n| kind(n) == "CompoundStmt")
                && !reserved(decl["name"].as_str().unwrap_or(""))
        })
        .collect();
    let mut translator = Translator {
        functions: definitions
            .iter()
            .filter_map(|f| f["name"].as_str().map(String::from))
            .collect(),
        records: Vec::new(),
        record_types: HashMap::new(),
        bitfields: HashMap::new(),
        mutated: HashSet::new(),
        cells: HashSet::new(),
        statics: HashMap::new(),
        globals: HashMap::new(),
        runtime: Vec::new(),
        scopes: Vec::new(),
        loops: Vec::new(),
        labels: HashMap::new(),
        fresh: 0,
    };
    translator.collect_records(ast);
    let globals = translator.collect_globals(ast);
    let mut functions = String::new();
    for function in &definitions {
        functions.push_str(&translator.function(function));
        functions.push('\n');
    }
    let globals = translator.global_definitions(&globals);
    let mut out = translator.struct_definitions();
    out.push_str(&globals);
    out.push_str(&functions);
    if let Some(main) = definitions.iter().find(|f| f["name"] == "main") {
        // Rust's own `main` calls the C one and exits with its result.
        let call = if inner(main).iter().any(|n| kind(n) == "ParmVarDecl") {
            translator.require(ARGS_RUNTIME);
            format!(
                "let argv = crt::args();\n    std::process::exit({}(argv.len() as i32 - 1, argv));",
                ident("main")
            )
        } else {
            format!("std::process::exit({}());", ident("main"))
        };
        out.push_str(&format!("fn main() {{\n    {call}\n}}\n"));
    }
    if !translator.runtime.is_empty() {
        out.push_str(
            "\n/// C library functions that Rust's standard library does not match.\nmod crt {\n",
        );
        out.push_str(&translator.runtime.join("\n"));
        out.push_str("}\n");
    }
    out
}

impl Translator {
    /// Records the structs that the program defines at file scope, by their
    /// tags and the typedefs that name them. A struct with an unnamed member
    /// or without a name to give it in Rust has no record.
    fn collect_records(&mut self, ast: &Value) {
        let mut ids = HashMap::new();
        for decl in inner(ast) {
            match kind(decl) {
                "RecordDecl"
                    if decl["tagUsed"] == "struct" && decl["completeDefinition"] == true =>
                {
                    let mut members = Vec::new();
                    for member in inner(decl).iter().filter(|n| kind(n) == "FieldDecl") {
                        let Some(name) = member["name"].as_str().filter(|name| !name.is_empty())
                        else {
                            members.clear();
                            break;
                        };
                        let width = (member["isBitfield"] == true)
                            .then(|| inner(member).first()?["value"].as_str()?.parse().ok())
                            .flatten();
                        if let (Some(width), Some(id)) = (width, member["id"].as_str()) {
                            self.bitfields.insert(id.to_string(), width);
                        }
                        members.push((name.to_string(), c_type(member).to_string(), width));
                    }
                    if members.is_empty() {
                        continue;
                    }
                    let tag = decl["name"].as_str().unwrap_or("");
                    if !tag.is_empty() {
                        self.record_types
                            .insert(format!("struct {tag}"), self.records.len());
                    }
                    if let Some(id) = decl["id"].as_str() {
                        ids.insert(id.to_string(), self.records.len());
                    }
                    self.records.push(Record {
                        name: tag.to_string(),
                        members,
                        used: std::cell::Cell::new(false),
                        assigned: std::cell::Cell::new(false),
                    });
                }
                "TypedefDecl" => {
                    let mut ty = inner(decl).first();
                    while let Some(node) = ty.filter(|node| kind(node) == "ElaboratedType") {
                        ty = inner(node).first();
                    }
                    let Some(&index) = ty
                        .filter(|node| kind(node) == "RecordType")
                        .and_then(|node| node["decl"]["id"].as_str())
                        .and_then(|id| ids.get(id))
                    else {
                        continue;
                    };
                    let name = decl["name"].as_str().unwrap_or("");
                    self.record_types.insert(name.to_string(), index);
                    if self.records[index].name.is_empty() {
                        self.records[index].name = name.to_string();
                    }
                }
                _ => {}
            }
        }
        // Each struct gets a Rust name of its own that no Rust type or path
        // of the translation has.
        let mut taken: HashSet<String> = RUST_NAMES.iter().map(|n| n.to_string()).collect();
        for record in &mut self.records {
            if record.name.is_empty() {
                continue;
            }
            let mut name = ident(&record.name);
            while !taken.insert(name.clone()) {
                name.push('_');
            }
            record.name = name;
        }
    }

    /// The record of the C struct type `c`, if the translation holds each of
    /// its members, so the translation names it and needs its definition.
    /// `div_t` and its kind are pairs instead.
    fn record(&self, c: &str) -> Option<&Record> {
        let record = &self.records[*self.record_types.get(c)?];
        if record.name.is_empty() || quotient_type(c).is_some() {
            return None;
        }
        for (_, ty, _) in &record.members {
            self.member_type(ty)?;
        }
        record.used.set(true);
        Some(record)
    }

    /// The Rust type of a struct member of the C type `c`: a cell of a
    /// number or `bool`, or another struct.
    fn member_type(&self, c: &str) -> Option<String> {
        match number_type(c) {
            Some(ty) => Some(format!("{CELL}<{ty}>")),
            None if is_bool(c) => Some(format!("{CELL}<bool>")),
            None => self.record(c).map(|record| record.name.clone()),
        }
    }

    /// The Rust definitions of the structs that the translation names. Each
    /// derives `Clone`, which copies it as C does, and `Default`, its value
    /// before C gives it one. One that the program assigns whole has a `set`
    /// that assigns each member, as C's `=` does, so a struct is assigned
    /// through a shared borrow as a cell is.
    fn struct_definitions(&self) -> String {
        // Assigning a struct assigns the structs of its members.
        let mut changed = true;
        while changed {
            changed = false;
            for record in self.records.iter().filter(|r| r.assigned.get()) {
                for (_, ty, _) in &record.members {
                    if let Some(member) = self.record(ty)
                        && !member.assigned.replace(true)
                    {
                        changed = true;
                    }
                }
            }
        }
        let mut out = String::new();
        let mut defined = 0;
        // Defining a struct names the structs of its members.
        while let Some(record) = self.records.iter().filter(|r| r.used.get()).nth(defined) {
            defined += 1;
            let mut fields = String::new();
            let mut sets = String::new();
            for (name, ty, _) in &record.members {
                let name = ident(name);
                let read = if self.record(ty).is_some() {
                    ""
                } else {
                    ".get()"
                };
                fields.push_str(&format!(
                    "    {name}: {},\n",
                    self.member_type(ty).unwrap_or_default()
                ));
                sets.push_str(&format!("        self.{name}.set(value.{name}{read});\n"));
            }
            let name = &record.name;
            out.push_str(&format!(
                "#[derive(Clone, Default)]\nstruct {name} {{\n{fields}}}\n\n"
            ));
            if record.assigned.get() {
                out.push_str(&format!(
                    "impl {name} {{\n    fn set(&self, value: {name}) {{\n{sets}    }}\n}}\n\n"
                ));
            }
        }
        out
    }

    /// The Rust type of a C type that the translation can hold in a
    /// variable: one of `builtin_type`, a struct or a pointer to a struct,
    /// which is `None` for null and otherwise the structs from the one it
    /// points to through the end of its array, a variable being an array of
    /// one.
    pub(crate) fn value_type(&self, c: &str) -> Option<String> {
        builtin_type(c).or_else(|| match c.strip_suffix(" *") {
            Some(pointee) => self
                .record(pointee)
                .map(|record| format!("Option<&[{}]>", record.name)),
            None => self.record(c).map(|record| record.name.clone()),
        })
    }

    /// The Rust type of a parameter or return value of the C type `c`. Every
    /// `char *` of a function borrows for its lifetime `'a`, so one it returns
    /// may come from a parameter, a string literal or an argument.
    fn signature_type(&self, c: &str) -> Option<String> {
        match c {
            "char *" => Some("Option<&'a [u8]>".to_string()),
            c => self.value_type(c),
        }
    }

    /// Whether `value_type` translates the C type `c` as a pointer.
    pub(crate) fn pointer(&self, c: &str) -> bool {
        c.ends_with(" *") && self.value_type(c).is_some()
    }

    /// The value of an uninitialised variable of a C type that `value_type`
    /// translates, as C gives the program no value to read.
    pub(crate) fn zero(&self, c: &str) -> String {
        let zero = if self.pointer(c) {
            "None"
        } else if is_bool(c) {
            "false"
        } else if c == "char **" {
            "&[]"
        } else if quotient_type(c).is_some() {
            "(0, 0)"
        } else if float_type(c).is_some() {
            "0.0"
        } else if let Some(record) = self.record(c) {
            return format!("{}::default()", record.name);
        } else {
            "0"
        };
        zero.to_string()
    }

    /// Emits `part` of the runtime into the translation.
    pub(crate) fn require(&mut self, part: &'static str) {
        if !self.runtime.contains(&part) {
            self.runtime.push(part);
        }
    }

    fn function(&mut self, function: &Value) -> String {
        let name = function["name"].as_str().unwrap_or("");
        let fn_type = function["type"]["qualType"].as_str().unwrap_or("");
        let ret = fn_type.split('(').next().unwrap_or("").trim();
        let ret = ret.trim_start_matches("const ");
        let params: Vec<&Value> = inner(function)
            .iter()
            .filter(|n| kind(n) == "ParmVarDecl")
            .collect();
        let Some(body) = inner(function).iter().find(|n| kind(n) == "CompoundStmt") else {
            return String::new();
        };
        let mut mutated = HashSet::new();
        collect_mutated(body, &mut mutated, &|c| self.record(c).is_some());
        self.mutated = mutated;
        self.cells.clear();
        collect_pointed_to(body, &mut self.cells);

        let borrows = ret == "char *" || params.iter().any(|p| c_type(p) == "char *");
        let mut out = format!("fn {}{}(", ident(name), if borrows { "<'a>" } else { "" });
        let argv = ["int", "char **"];
        if name == "main" && !params.is_empty() && !params.iter().map(|p| c_type(p)).eq(argv) {
            out.push_str(&format!(
                ") {{\n    {};\n}}\n",
                unsupported("main with parameters")
            ));
            return out;
        }
        // A parameter that the program points to moves into a cell.
        let cells: String = params
            .iter()
            .filter(|p| self.is_cell(p["id"].as_str()))
            .map(|p| {
                let name = ident(p["name"].as_str().unwrap_or(""));
                format!("    let {name} = {CELL}::new({name});\n")
            })
            .collect();
        let params: Vec<String> = params
            .iter()
            .map(|p| self.typed_binding(p, self.signature_type(c_type(p))))
            .collect();
        out.push_str(&params.join(", "));
        out.push(')');
        match ret {
            "void" => {}
            // Rust has no lifetime for the variable such a pointer borrows.
            ret if self.pointer(ret) && ret != "char *" => {
                out.push_str(&format!(
                    " -> {}",
                    unsupported(&format!("return type {ret}"))
                ));
            }
            ret => match self.signature_type(ret) {
                Some(ty) => out.push_str(&format!(" -> {ty}")),
                None => out.push_str(&format!(" -> {}", unsupported(&format!("type {ret}")))),
            },
        }
        out.push_str(" {\n");
        out.push_str(&cells);
        let stmts = inner(body);
        let (last, init) = match stmts.split_last() {
            Some((last, init)) if kind(last) == "ReturnStmt" && !inner(last).is_empty() => {
                (Some(&inner(last)[0]), init)
            }
            _ => (None, stmts),
        };
        self.items(init, 1, &mut out);
        // The last `return` is the tail expression, and `main` returns 0 when
        // it runs off its end.
        match last {
            Some(value) => out.push_str(&format!("    {}\n", self.expr(value))),
            None if name == "main" => out.push_str("    0\n"),
            None => {}
        }
        out.push_str("}\n");
        out
    }

    /// `name: Type` for a parameter, with `mut` if the function assigns it.
    pub(crate) fn binding(&self, var: &Value) -> String {
        self.typed_binding(var, self.value_type(c_type(var)))
    }

    /// `binding` with the Rust type `ty`, if the translation has one.
    fn typed_binding(&self, var: &Value, ty: Option<String>) -> String {
        let ty = ty.unwrap_or_else(|| unsupported(&format!("type {}", c_type(var))));
        format!(
            "{}{}: {ty}",
            self.mut_(var),
            ident(var["name"].as_str().unwrap_or(""))
        )
    }

    /// `mut` if the function assigns the variable other than through its cell.
    fn mut_(&self, var: &Value) -> &'static str {
        let id = var["id"].as_str();
        if id.is_some_and(|id| self.mutated.contains(id)) && !self.is_cell(id) {
            "mut "
        } else {
            ""
        }
    }

    /// Whether the variable with the id `id` is held in a cell.
    pub(crate) fn is_cell(&self, id: Option<&str>) -> bool {
        id.is_some_and(|id| self.cells.contains(id))
    }

    /// The `let` of a local variable that `value_type` translates, with the
    /// Rust value `init`, in a cell if the program points to it.
    pub(crate) fn declaration(&self, var: &Value, init: &str) -> String {
        match rust_type(c_type(var)) {
            Some(ty) if self.is_cell(var["id"].as_str()) => format!(
                "let {}: {CELL}<{ty}> = {CELL}::new({init});",
                ident(var["name"].as_str().unwrap_or(""))
            ),
            _ => format!("let {} = {init};", self.binding(var)),
        }
    }

    pub(crate) fn stmt(&mut self, stmt: &Value, depth: usize, out: &mut String) {
        let pad = "    ".repeat(depth);
        if self.control(stmt, depth, out) {
            return;
        }
        match kind(stmt) {
            "CompoundStmt" => {
                out.push_str(&format!("{pad}{{\n"));
                self.items(inner(stmt), depth + 1, out);
                out.push_str(&format!("{pad}}}\n"));
            }
            "DeclStmt" => {
                for var in inner(stmt) {
                    out.push_str(&pad);
                    out.push_str(&self.local(var));
                    out.push('\n');
                }
            }
            "ReturnStmt" => match inner(stmt).first() {
                Some(value) => out.push_str(&format!("{pad}return {};\n", self.expr(value))),
                None => out.push_str(&format!("{pad}return;\n")),
            },
            "NullStmt" => {}
            _ => out.push_str(&format!("{pad}{};\n", self.expr_stmt(stmt))),
        }
    }

    fn local(&mut self, var: &Value) -> String {
        if kind(var) != "VarDecl" {
            return format!("{};", unsupported(&format!("declaration {}", kind(var))));
        }
        match var["storageClass"].as_str() {
            Some("static") => return self.static_local(var),
            // It names an object defined elsewhere, not a fresh zero.
            Some("extern") => return format!("{};", unsupported("extern local variable")),
            _ => {}
        }
        if let Some((element, length)) = array_type(c_type(var)) {
            return self.array(var, element, length);
        }
        // An uninitialised C variable has no value to read, so any value will do.
        let init = match inner(var).first() {
            Some(init) if var["init"].is_string() => self.expr(init),
            _ => self.zero(c_type(var)),
        };
        self.declaration(var, &init)
    }

    /// A local array of integers as cells, which pointers into it share: a
    /// borrowed Rust array for a fixed length and a `Vec` for a variable
    /// one. Clang gives a variable length only as the text of its
    /// expression, so it must be a variable's name. Elements without an
    /// initialiser are 0, as C gives no value to read. An array of `char`
    /// is bytes borrowed mutably instead, which `char *` borrows to read.
    fn array(&mut self, var: &Value, element: &str, length: &str) -> String {
        let name = ident(var["name"].as_str().unwrap_or(""));
        let bytes = element == "char";
        let (slice, borrow, zero) = match cell_type(element) {
            _ if bytes => ("&mut [u8]".to_string(), "&mut ", "0".to_string()),
            Some(ty) => (format!("&[{CELL}<{ty}>]"), "&", format!("{CELL}::new(0)")),
            None => return format!("{};", unsupported(&format!("type {}", c_type(var)))),
        };
        let init = inner(var).first().filter(|_| var["init"].is_string());
        if fixed_length(length) {
            // As a slice, so a constant index out of bounds panics when it
            // runs, as an array's would fail to compile even where it never
            // runs.
            return match self.fixed_array(var, bytes, &zero, length) {
                Ok(value) => format!("let {name}: {slice} = {borrow}{value};"),
                Err(err) => format!("{err};"),
            };
        }
        let mut chars = length.chars();
        let variable = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !variable || init.is_some() {
            return format!(
                "{};",
                unsupported(&format!("variable length array {}", c_type(var)))
            );
        }
        // A negative length converts to one too large to allocate.
        format!(
            "let {name}: {slice} = {borrow}vec![{zero}; {} as usize];",
            ident(length)
        )
    }

    /// The Rust array of `length` elements, bytes or else cells with the
    /// value `zero`, that the initialiser of `var` gives, if it has one.
    fn fixed_array(
        &mut self,
        var: &Value,
        bytes: bool,
        zero: &str,
        length: &str,
    ) -> Result<String, String> {
        let array = if bytes {
            format!("[0; {length}]")
        } else {
            format!("[const {{ {zero} }}; {length}]")
        };
        Ok(
            match inner(var).first().filter(|_| var["init"].is_string()) {
                None => array,
                // The literal's bytes, without the zero at its end when the
                // array has no room for it, and zeros after them.
                Some(literal) if bytes && let Some(mut text) = stdio::string_array(literal) => {
                    text.truncate(length.parse().unwrap_or(usize::MAX));
                    while text.last() == Some(&0) {
                        text.pop();
                    }
                    if text.is_empty() {
                        array
                    } else {
                        format!(
                            "{{ let mut __array = {array}; __array[..{}].copy_from_slice({}); __array }}",
                            text.len(),
                            stdio::byte_string(&text)
                        )
                    }
                }
                Some(list) if kind(list) == "InitListExpr" => {
                    // With a filler for the elements it leaves out, Clang lists
                    // the filler and then the elements.
                    let elements = match list["array_filler"].as_array() {
                        Some(filler) => &filler[1..],
                        None => inner(list),
                    };
                    let mut sets = String::new();
                    for (i, element) in elements.iter().enumerate() {
                        if kind(element) == "ImplicitValueInitExpr" {
                            continue;
                        }
                        let set = if bytes {
                            format!(" __array[{i}] = {} as u8;", self.operand(element))
                        } else {
                            format!(" __array[{i}].set({});", self.expr(element))
                        };
                        sets.push_str(&set);
                    }
                    let binding = if bytes { "let mut" } else { "let" };
                    if sets.is_empty() {
                        array
                    } else {
                        format!("{{ {binding} __array = {array};{sets} __array }}")
                    }
                }
                Some(_) => return Err(unsupported("array initialiser")),
            },
        )
    }

    /// Records the global variables that the program defines and the
    /// translation holds, by the ids of all their declarations, as C lets
    /// a file declare one again. Returns the declaration that defines each,
    /// the one with an initialiser if there is one.
    fn collect_globals<'v>(&mut self, ast: &'v Value) -> Vec<&'v Value> {
        let vars: Vec<&Value> = inner(ast)
            .iter()
            .filter(|decl| kind(decl) == "VarDecl")
            .collect();
        let mut definitions: Vec<&Value> = Vec::new();
        // Declarations from headers have no initialiser and are `extern`.
        for &var in vars.iter().filter(|var| var["storageClass"] != "extern") {
            match definitions.iter_mut().find(|d| d["name"] == var["name"]) {
                Some(definition) if var["init"].is_string() => *definition = var,
                Some(_) => {}
                None => definitions.push(var),
            }
        }
        for definition in &definitions {
            if self.global_type(c_type(definition)).is_none() {
                continue;
            }
            let name = definition["name"].as_str().unwrap_or("");
            let array = array_type(c_type(definition)).is_some();
            for var in vars.iter().filter(|var| var["name"] == name) {
                if let Some(id) = var["id"].as_str() {
                    let function = format!("__global_{name}");
                    self.globals.insert(id.to_string(), (function, array));
                }
            }
        }
        definitions
    }

    /// The Rust type of a global variable of the C type `c` that the
    /// translation holds, which it borrows for the whole program: a cell of
    /// a number or `bool`, a struct or a slice of cells for an integer array
    /// of a fixed length, as R10 and R13 hold local ones.
    fn global_type(&self, c: &str) -> Option<String> {
        if let Some((element, length)) = array_type(c) {
            let ty = cell_type(element).filter(|_| fixed_length(length))?;
            return Some(format!("[{CELL}<{ty}>]"));
        }
        match self.record(c) {
            Some(record) => Some(record.name.clone()),
            None if is_bool(c) => Some(format!("{CELL}<bool>")),
            None => number_type(c).map(|ty| format!("{CELL}<{ty}>")),
        }
    }

    /// A Rust function for each global variable in `definitions` that
    /// gives it, made on its first use with C's initial value, which is
    /// constant. Safe Rust has no mutable static of a cell, so each thread
    /// holds its own, leaked to borrow it for the whole program; the
    /// program's one thread is all that uses it.
    fn global_definitions(&mut self, definitions: &[&Value]) -> String {
        self.cells.clear();
        self.mutated.clear();
        let mut out = String::new();
        for definition in definitions {
            let ty = c_type(definition);
            let Some(rust) = self.global_type(ty) else {
                out.push_str(&unsupported(&format!("global variable of type {ty}")));
                out.push_str(";\n\n");
                continue;
            };
            let value = match array_type(ty) {
                Some((_, length)) => {
                    let zero = format!("{CELL}::new(0)");
                    self.fixed_array(definition, false, &zero, length)
                        .unwrap_or_else(|err| err)
                }
                None => {
                    let value = match inner(definition).first() {
                        Some(init) if definition["init"].is_string() => self.expr(init),
                        _ => self.zero(ty),
                    };
                    if self.record(ty).is_some() {
                        value
                    } else {
                        format!("{CELL}::new({value})")
                    }
                }
            };
            let name = definition["name"].as_str().unwrap_or("");
            out.push_str(&format!(
                "fn __global_{name}() -> &'static {rust} {{\n    thread_local! {{\n        static GLOBAL: &'static {rust} = Box::leak(Box::new({value}));\n    }}\n    GLOBAL.with(|global| *global)\n}}\n\n"
            ));
        }
        out
    }

    /// The Rust place of the global variable that `expr` names, if the
    /// translation holds it: the slice of an array, as a local array is a
    /// borrowed slice, and the value of anything else.
    fn global(&self, expr: &Value) -> Option<String> {
        if kind(expr) != "DeclRefExpr" {
            return None;
        }
        let (function, array) = self.globals.get(expr["referencedDecl"]["id"].as_str()?)?;
        Some(if *array {
            format!("{function}()")
        } else {
            format!("(*{function}())")
        })
    }

    /// A static local variable of an integer type, as a Rust static atomic
    /// that only its block sees. C initialises it once, to zero if the
    /// program gives no value, and the atomic lets safe Rust change it.
    fn static_local(&mut self, var: &Value) -> String {
        let ty = c_type(var);
        let Some(rust) = rust_type(ty) else {
            return format!(
                "{};",
                unsupported(&format!("static local variable of type {ty}"))
            );
        };
        // C reserves names that start with `__`, so no C name meets it, and
        // Rust lets no `let` bind the name of a static in scope.
        let name = format!(
            "__{}_{}",
            var["name"].as_str().unwrap_or("").to_uppercase(),
            self.fresh
        );
        self.fresh += 1;
        let init = match inner(var).first() {
            Some(init) if var["init"].is_string() => self.expr(init),
            _ => self.zero(ty),
        };
        if let Some(id) = var["id"].as_str() {
            self.statics.insert(id.to_string(), name.clone());
        }
        let atomic = format!("std::sync::atomic::Atomic{}", rust.to_uppercase());
        format!("static {name}: {atomic} = {atomic}::new({init});")
    }

    /// The Rust static of a static local variable that `expr` names, or of
    /// `errno`.
    fn static_var(&mut self, expr: &Value) -> Option<String> {
        let mut expr = expr;
        while kind(expr) == "ParenExpr" {
            expr = &inner(expr)[0];
        }
        if errno(expr) {
            self.require(ERRNO_RUNTIME);
            return Some("crt::ERRNO".to_string());
        }
        if kind(expr) != "DeclRefExpr" {
            return None;
        }
        let id = expr["referencedDecl"]["id"].as_str()?;
        self.statics.get(id).cloned()
    }

    /// Assigns `value` to `target`, or with a binary operator such as `+`,
    /// the result of applying it to `target` and `value`.
    fn assign(&mut self, target: &Value, op: &str, value: &Value) -> String {
        let ty = c_type(target);
        match self.static_var(target) {
            Some(name) if op.is_empty() => format!("{name}.store({}, {RELAXED})", self.expr(value)),
            Some(name) => {
                let value = self.right_operand(op, ty, value);
                let result = binary(op, ty, &format!("{name}.load({RELAXED})"), &value);
                format!("{name}.store({result}, {RELAXED})")
            }
            None if op.is_empty() => {
                let value = self.expr(value);
                self.store(target, &value)
            }
            None if let Some(cell) = self.cell(target) => {
                let value = self.right_operand(op, ty, value);
                let width = self.bitfield(target);
                update(&cell, |old| {
                    truncate(width, ty, &binary(op, ty, old, &value))
                })
            }
            // Rust's `+=` would check what C wraps.
            None if wraps(op, ty) => {
                let place = self.place(target);
                let value = self.right_operand(op, ty, value);
                format!("{place} = {}", binary(op, ty, &place, &value))
            }
            None => format!("{} {op}= {}", self.place(target), self.expr(value)),
        }
    }

    /// An expression whose value C discards, where assignments and output
    /// calls have their Rust statement forms.
    pub(crate) fn expr_stmt(&mut self, expr: &Value) -> String {
        let args = inner(expr);
        match (kind(expr), expr["opcode"].as_str()) {
            ("BinaryOperator", Some("=")) => self.assign(&args[0], "", &args[1]),
            ("CompoundAssignOperator", Some(op)) => {
                let ty = c_type(&args[0]);
                let op = op.trim_end_matches('=');
                let supported = match float_type(ty) {
                    Some(_) => matches!(op, "+" | "-" | "*" | "/"),
                    None => rust_type(ty).is_some() && ARITHMETIC.contains(&op),
                };
                // A `float` that C computes in `double` would need converting.
                if ty != expr["computeLHSType"]["qualType"] || !supported {
                    return unsupported(&format!("{op}= on {ty}"));
                }
                self.assign(&args[0], op, &args[1])
            }
            ("UnaryOperator", Some(op @ ("++" | "--"))) => {
                let (assign, method) = if op == "++" {
                    ("+=", "wrapping_add")
                } else {
                    ("-=", "wrapping_sub")
                };
                let ty = c_type(&args[0]);
                if rust_type(ty).is_none() {
                    return unsupported(&format!("{op} on {ty}"));
                }
                // Overflow is undefined for `int` and `long`, so Rust may
                // check it. Unsigned types wrap, and narrower ones convert
                // back from `int`, which wraps on x86-64 too.
                let checked = matches!(ty, "int" | "long" | "long long");
                let step = |value: &str| {
                    if checked {
                        format!("{value} {} 1", &assign[..1])
                    } else {
                        format!("{value}.{method}(1)")
                    }
                };
                if let Some(name) = self.static_var(&args[0]) {
                    let value = step(&format!("{name}.load({RELAXED})"));
                    return format!("{name}.store({value}, {RELAXED})");
                }
                if let Some(cell) = self.cell(&args[0]) {
                    let width = self.bitfield(&args[0]);
                    return update(&cell, |old| truncate(width, ty, &step(old)));
                }
                let place = self.place(&args[0]);
                if checked {
                    format!("{place} {assign} 1")
                } else {
                    format!("{place} = {place}.{method}(1)")
                }
            }
            ("CallExpr", _) => match callee(expr) {
                Some("printf") => stdio::printf(self, "print", &args[1..]),
                Some("fprintf") => match args.get(1).and_then(stdio::stream) {
                    Some("stdout") => stdio::printf(self, "print", &args[2..]),
                    Some("stderr") => stdio::printf(self, "eprint", &args[2..]),
                    _ => unsupported("fprintf to a stream other than stdout or stderr"),
                },
                _ => self.expr(expr),
            },
            ("ParenExpr", _) => self.expr_stmt(&args[0]),
            _ => self.expr(expr),
        }
    }

    /// Writes the Rust value `value` to the C lvalue `target`.
    pub(crate) fn store(&mut self, target: &Value, value: &str) -> String {
        if let Some(record) = self.record(c_type(target)) {
            record.assigned.set(true);
            return format!("{}.set({value})", self.operand(target));
        }
        if let Some(byte) = self.byte(target) {
            return format!("{byte} = ({value}) as u8");
        }
        match self.cell(target) {
            Some(cell) => {
                let value = truncate(self.bitfield(target), c_type(target), value);
                format!("{cell}.set({value})")
            }
            None => format!("{} = {value}", self.place(target)),
        }
    }

    /// The width of the bit-field that `expr` designates, if it is one.
    fn bitfield(&self, expr: &Value) -> Option<u32> {
        let mut expr = expr;
        while kind(expr) == "ParenExpr" {
            expr = &inner(expr)[0];
        }
        let member = expr["referencedMemberDecl"].as_str()?;
        (kind(expr) == "MemberExpr")
            .then(|| self.bitfields.get(member).copied())
            .flatten()
    }

    /// The Rust place of a member of a struct, `s.m` or `p->m`, where `p`
    /// points to the first of its structs.
    fn member(&mut self, expr: &Value) -> Option<String> {
        let base = &inner(expr)[0];
        let base = if expr["isArrow"] == true {
            format!("{}[0]", self.elements(base)?)
        } else {
            self.record(c_type(base))?;
            self.operand(base)
        };
        Some(format!("{base}.{}", ident(expr["name"].as_str()?)))
    }

    /// The Rust place of a struct that `expr` designates as `*p`, `p[i]`,
    /// `s.m` or `p->m`.
    fn struct_place(&mut self, expr: &Value) -> Option<String> {
        let args = inner(expr);
        match kind(expr) {
            "ArraySubscriptExpr" if rust_type(c_type(&args[1])).is_some() => {
                let elements = self.elements(&args[0])?;
                Some(format!("{elements}[{}]", self.index(&args[1])))
            }
            "UnaryOperator" if expr["opcode"] == "*" => {
                Some(format!("{}[0]", self.elements(&args[0])?))
            }
            "MemberExpr" => self.member(expr),
            _ => None,
        }
    }

    /// A struct of the C type `c` from the elements of the initialiser list
    /// `list`, which Clang gives one for each member in order, those without
    /// a value as `ImplicitValueInitExpr`.
    fn struct_literal(&mut self, c: &str, list: &Value) -> String {
        let Some(record) = self.record(c) else {
            return unsupported(&format!("initialiser of {c}"));
        };
        let (name, members) = (record.name.clone(), record.members.clone());
        let elements = inner(list);
        if elements.len() > members.len() || !list["array_filler"].is_null() {
            return unsupported(&format!("initialiser of {c}"));
        }
        let mut fields = Vec::new();
        for (i, (member, ty, width)) in members.iter().enumerate() {
            let value = match elements.get(i) {
                Some(element) if kind(element) != "ImplicitValueInitExpr" => {
                    let value = self.expr(element);
                    truncate(*width, ty, &value)
                }
                _ => self.zero(ty),
            };
            let value = if self.record(ty).is_some() {
                value
            } else {
                format!("{CELL}::new({value})")
            };
            fields.push(format!("{}: {value}", ident(member)));
        }
        format!("{name} {{ {} }}", fields.join(", "))
    }

    /// The Rust place of a C `char` that is an element of a local `char`
    /// array, `a[i]` with an integer index, as one of the array's bytes.
    fn byte(&mut self, expr: &Value) -> Option<String> {
        let args = inner(expr);
        match kind(expr) {
            "ParenExpr" => self.byte(&args[0]),
            "ArraySubscriptExpr"
                if c_type(expr) == "char" && rust_type(c_type(&args[1])).is_some() =>
            {
                let array = self.buffer(&args[0])?;
                Some(format!("{array}[{}]", self.index(&args[1])))
            }
            _ => None,
        }
    }

    /// The local `char` array whose decay `pointer` is, perhaps converted to
    /// `const char *` or `void *`, as its Rust name.
    fn buffer(&mut self, pointer: &Value) -> Option<String> {
        let mut expr = pointer;
        while kind(expr) == "ParenExpr"
            || (matches!(kind(expr), "ImplicitCastExpr" | "CStyleCastExpr")
                && matches!(expr["castKind"].as_str(), Some("NoOp" | "BitCast")))
        {
            expr = &inner(expr)[0];
        }
        let array = inner(expr).first()?;
        (kind(expr) == "ImplicitCastExpr"
            && expr["castKind"] == "ArrayToPointerDecay"
            && variable(array)
            && char_array(c_type(array)))
        .then(|| self.expr(array))
    }

    /// The Rust cell of a C lvalue that the translation holds in one: an
    /// integer variable that the program points to, an element of an
    /// integer array, `*p` of a pointer to an integer or a struct member of
    /// a scalar type.
    fn cell(&mut self, expr: &Value) -> Option<String> {
        let args = inner(expr);
        // A struct is no cell, but its members of scalar types are.
        if self.record(c_type(expr)).is_some() {
            return None;
        }
        match kind(expr) {
            "ParenExpr" => self.cell(&args[0]),
            "MemberExpr" => self.member(expr),
            // A global array is no cell, but its elements are.
            "DeclRefExpr" if self.global(expr).is_some() => self
                .global(expr)
                .filter(|_| array_type(c_type(expr)).is_none()),
            "DeclRefExpr"
                if self.static_var(expr).is_none()
                    && self.is_cell(expr["referencedDecl"]["id"].as_str()) =>
            {
                Some(ident(expr["referencedDecl"]["name"].as_str().unwrap_or("")))
            }
            "ArraySubscriptExpr" if rust_type(c_type(&args[1])).is_some() => {
                let elements = self.elements(&args[0])?;
                Some(format!("{elements}[{}]", self.index(&args[1])))
            }
            "UnaryOperator" if expr["opcode"] == "*" && !errno(expr) => {
                Some(format!("{}[0]", self.elements(&args[0])?))
            }
            _ => None,
        }
    }

    /// The cells or structs that a pointer to an integer or a struct points
    /// to, from the first: the array that decays to it, or the pointer's
    /// slice, as C has no object at null.
    fn elements(&mut self, pointer: &Value) -> Option<String> {
        let pointee = c_type(pointer).strip_suffix(" *")?;
        if cell_type(pointee).is_none() {
            self.record(pointee)?;
        }
        let mut decayed = pointer;
        while kind(decayed) == "ParenExpr" {
            decayed = &inner(decayed)[0];
        }
        if kind(decayed) == "ImplicitCastExpr"
            && decayed["castKind"] == "ArrayToPointerDecay"
            && variable(&inner(decayed)[0])
        {
            return Some(self.expr(&inner(decayed)[0]));
        }
        Some(format!(
            "{}.expect(\"null pointer dereference\")",
            self.operand(pointer)
        ))
    }

    /// An integer index into a slice. A negative one converts to one beyond
    /// any slice, so it panics as an index out of bounds does.
    fn index(&mut self, index: &Value) -> String {
        match kind(index) {
            "IntegerLiteral" => index["value"].as_str().unwrap_or("").to_string(),
            _ => format!("{} as usize", self.operand(index)),
        }
    }

    /// An lvalue in a position Rust writes to.
    pub(crate) fn place(&mut self, expr: &Value) -> String {
        match kind(expr) {
            // Rust writes a static atomic with `store`.
            "DeclRefExpr" if self.static_var(expr).is_some() => {
                unsupported("static local variable in this position")
            }
            "DeclRefExpr" => self.expr(expr),
            "ParenExpr" => self.place(&inner(expr)[0]),
            other => unsupported(&format!("assignment to {other}")),
        }
    }

    pub(crate) fn expr(&mut self, expr: &Value) -> String {
        let args = inner(expr);
        match kind(expr) {
            "IntegerLiteral" => {
                let value = expr["value"].as_str().unwrap_or("");
                match rust_type(c_type(expr)) {
                    Some("i32") => value.to_string(),
                    Some(ty) => format!("{value}_{ty}"),
                    None => unsupported(&format!("literal of type {}", c_type(expr))),
                }
            }
            // Clang gives the value with the digits that tell it apart from
            // any other of its type, and Rust prints it with the fewest.
            "FloatingLiteral" => {
                let value = expr["value"].as_str().unwrap_or("");
                let text = match float_type(c_type(expr)) {
                    Some("f32") => value
                        .parse::<f32>()
                        .ok()
                        .filter(|v| v.is_finite())
                        .map(|v| format!("{v:?}_f32")),
                    Some(_) => value
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite())
                        .map(|v| format!("{v:?}_f64")),
                    None => None,
                };
                text.unwrap_or_else(|| unsupported(&format!("literal of type {}", c_type(expr))))
            }
            // Clang gives the value as the bits of an unsigned int. A negative
            // one has a suffix, or `as u32` would make Rust read it unsigned.
            "CharacterLiteral" if c_type(expr) == "int" => match expr["value"].as_u64() {
                Some(value) if value <= i32::MAX as u64 => value.to_string(),
                Some(value) => format!("{}_i32", value as u32 as i32),
                None => unsupported("character literal"),
            },
            "DeclRefExpr" => match self.static_var(expr) {
                Some(name) => format!("{name}.load({RELAXED})"),
                None => match self.cell(expr) {
                    Some(cell) => format!("{cell}.get()"),
                    // A global struct or array.
                    None => self.global(expr).unwrap_or_else(|| {
                        ident(expr["referencedDecl"]["name"].as_str().unwrap_or(""))
                    }),
                },
            },
            _ if errno(expr) => {
                self.require(ERRNO_RUNTIME);
                format!("crt::ERRNO.load({RELAXED})")
            }
            // The size of a number or a pointer on x86-64 Linux.
            "UnaryExprOrTypeTraitExpr" if expr["name"] == "sizeof" => {
                let of = match args.first() {
                    Some(operand) => c_type(operand),
                    None => qual_type(&expr["argType"]),
                };
                let size = |of: &str| match number_type(of) {
                    _ if of.ends_with('*') => Some(8),
                    Some(ty) => ty[1..].parse::<u64>().ok().map(|bits| bits / 8),
                    None if is_bool(of) => Some(1),
                    None => None,
                };
                // An array of a fixed length holds that many elements.
                let size = match array_type(of) {
                    Some((element, length)) => length
                        .parse::<u64>()
                        .ok()
                        .zip(size(element))
                        .and_then(|(length, size)| length.checked_mul(size)),
                    None => size(of),
                };
                let Some(size) = size else {
                    return unsupported(&format!("sizeof {of}"));
                };
                match rust_type(c_type(expr)) {
                    Some(ty) => format!("{size}_{ty}"),
                    None => unsupported(&format!("sizeof of type {}", c_type(expr))),
                }
            }
            "ConditionalOperator" => format!(
                "if {} {{ {} }} else {{ {} }}",
                self.cond(&args[0]),
                self.expr(&args[1]),
                self.expr(&args[2])
            ),
            // A struct as a place, which a value of it copies.
            _ if self.record(c_type(expr)).is_some()
                && let Some(place) = self.struct_place(expr) =>
            {
                place
            }
            "InitListExpr" => self.struct_literal(c_type(expr), expr),
            // An element of `argv`.
            "ArraySubscriptExpr"
                if c_type(&args[0]) == "char **" && rust_type(c_type(&args[1])).is_some() =>
            {
                format!("{}[{}]", self.operand(&args[0]), self.index(&args[1]))
            }
            "ArraySubscriptExpr" if let Some(byte) = self.byte(expr) => format!("{byte} as i8"),
            // A byte that a `char *` points into; one past the end of its
            // array panics.
            "ArraySubscriptExpr"
                if c_type(&args[0]) == "char *" && rust_type(c_type(&args[1])).is_some() =>
            {
                format!(
                    "{}.expect(\"null pointer dereference\")[{}] as i8",
                    self.operand(&args[0]),
                    self.index(&args[1])
                )
            }
            "ArraySubscriptExpr" => match self.cell(expr) {
                Some(cell) => format!("{cell}.get()"),
                None => unsupported("ArraySubscriptExpr"),
            },
            "ParenExpr" => self.expr(&args[0]),
            "ImplicitCastExpr" | "CStyleCastExpr" => self.cast(expr),
            // A member of the result of `div`, which is held as `(quot, rem)`.
            "MemberExpr"
                if expr["isArrow"] == false && quotient_type(c_type(&args[0])).is_some() =>
            {
                let field = match expr["name"].as_str() {
                    Some("quot") => 0,
                    Some("rem") => 1,
                    _ => return unsupported("member of div_t"),
                };
                format!("{}.{field}", self.operand(&args[0]))
            }
            "MemberExpr" => match self.cell(expr) {
                Some(cell) => format!("{cell}.get()"),
                None => unsupported("MemberExpr"),
            },
            _ if control::boolean(expr) => format!("i32::from({})", self.cond(expr)),
            "UnaryOperator" => match expr["opcode"].as_str() {
                Some("-") if signed(c_type(expr)) || float_type(c_type(expr)).is_some() => {
                    format!("-{}", self.operand(&args[0]))
                }
                // C negates an unsigned value modulo its range.
                Some("-") if rust_type(c_type(expr)).is_some() => {
                    format!("{}.wrapping_neg()", self.operand(&args[0]))
                }
                Some("~") if rust_type(c_type(expr)).is_some() => {
                    format!("!{}", self.operand(&args[0]))
                }
                Some("+") => self.expr(&args[0]),
                // The address of an integer variable or member, the one cell
                // it holds, or of a struct, or of an array element, the cells
                // or structs from it to the array's end, which may be none
                // past its last element.
                Some("&") if c_type(expr) != "char *" && self.pointer(c_type(expr)) => {
                    let mut target = &args[0];
                    while kind(target) == "ParenExpr" {
                        target = &inner(target)[0];
                    }
                    let structure = self.record(c_type(target)).is_some();
                    match (kind(target), self.cell(target)) {
                        ("DeclRefExpr" | "MemberExpr", Some(cell)) => {
                            format!("Some(std::slice::from_ref(&{cell}))")
                        }
                        ("DeclRefExpr" | "MemberExpr", None) if structure => {
                            format!("Some(std::slice::from_ref(&{}))", self.expr(target))
                        }
                        ("ArraySubscriptExpr", cell) if cell.is_some() || structure => {
                            let target = inner(target);
                            let elements = self.elements(&target[0]).unwrap_or_default();
                            format!("Some(&{elements}[{}..])", self.index(&target[1]))
                        }
                        _ => unsupported("unary &"),
                    }
                }
                Some("*") if c_type(&args[0]) != "char *" && self.pointer(c_type(&args[0])) => {
                    match self.cell(expr) {
                        Some(cell) => format!("{cell}.get()"),
                        None => unsupported("unary *"),
                    }
                }
                // The byte a `char *` points to, the first of its slice; one
                // past the end of the array has none, and indexing panics.
                Some("*") if c_type(&args[0]) == "char *" => format!(
                    "{}.expect(\"null pointer dereference\")[0] as i8",
                    self.operand(&args[0])
                ),
                op => unsupported(&format!("unary {}", op.unwrap_or("?"))),
            },
            "BinaryOperator" => match expr["opcode"].as_str() {
                Some(op) if ARITHMETIC.contains(&op) && rust_type(c_type(expr)).is_some() => {
                    let left = self.operand(&args[0]);
                    let right = self.right_operand(op, c_type(expr), &args[1]);
                    binary(op, c_type(expr), &left, &right)
                }
                // Rust's floating operators round as C's do, and give an
                // infinity or not a number where C's do.
                Some(op @ ("+" | "-" | "*" | "/")) if float_type(c_type(expr)).is_some() => {
                    let left = self.operand(&args[0]);
                    format!("{left} {op} {}", self.operand(&args[1]))
                }
                op => unsupported(&format!("binary {} on {}", op.unwrap_or("?"), c_type(expr))),
            },
            "CallExpr" => match callee(expr) {
                Some(name) if self.functions.contains(name) => {
                    let args: Vec<String> = args[1..].iter().map(|a| self.expr(a)).collect();
                    format!("{}({})", ident(name), args.join(", "))
                }
                Some("div" | "ldiv" | "lldiv") => {
                    self.require(DIV_RUNTIME);
                    format!("crt::div({}, {})", self.expr(&args[1]), self.expr(&args[2]))
                }
                Some(name @ ("strtol" | "strtoll" | "strtoul" | "strtoull")) => {
                    self.strtol(name, &args[1..])
                }
                // glibc's `atoi` is `strtol` in base 10 converted to `int`,
                // and its `atol` and `atoll` are `strtol` itself.
                Some(name @ ("atoi" | "atol" | "atoll")) => {
                    self.require(ERRNO_RUNTIME);
                    self.require(ERANGE_RUNTIME);
                    self.require(STRTOL_RUNTIME);
                    self.require(STRTOL_SIGNED_RUNTIME);
                    let s = self.expr(&args[1]);
                    if name == "atoi" {
                        self.require(ATOI_RUNTIME);
                        format!("crt::atoi({s})")
                    } else {
                        format!("crt::strtol({s}, None, 10)")
                    }
                }
                // glibc's `atof` is `strtod` without an end pointer.
                Some("atof") => {
                    self.require(ERRNO_RUNTIME);
                    self.require(ERANGE_RUNTIME);
                    self.require(STRTOD_RUNTIME);
                    format!("crt::strtod({}, None)", self.expr(&args[1]))
                }
                Some("strtod") => match self.end_pointer("strtod", &args[2]) {
                    Ok(end) => {
                        self.require(ERRNO_RUNTIME);
                        self.require(ERANGE_RUNTIME);
                        self.require(STRTOD_RUNTIME);
                        format!("crt::strtod({}, {end})", self.expr(&args[1]))
                    }
                    Err(err) => err,
                },
                // Rust's methods compute these exactly, as glibc does, and
                // none sets `errno`.
                Some(name @ ("fabs" | "floor" | "ceil")) => {
                    let method = if name == "fabs" { "abs" } else { name };
                    format!("{}.{method}()", self.operand(&args[1]))
                }
                Some("sqrt") => {
                    self.require(ERRNO_RUNTIME);
                    self.require(EDOM_RUNTIME);
                    self.require(SQRT_RUNTIME);
                    format!("crt::sqrt({})", self.expr(&args[1]))
                }
                Some("pow") => {
                    self.require(ERRNO_RUNTIME);
                    self.require(EDOM_RUNTIME);
                    self.require(ERANGE_RUNTIME);
                    self.require(POW_RUNTIME);
                    format!("crt::pow({}, {})", self.expr(&args[1]), self.expr(&args[2]))
                }
                Some("strlen") => {
                    self.require(STRLEN_RUNTIME);
                    format!("crt::strlen({})", self.expr(&args[1]))
                }
                Some(name @ ("fgets" | "memset" | "strcpy" | "strncpy")) => {
                    self.write_bytes(name, &args[1..])
                }
                Some("rand") if args.len() == 1 => {
                    self.require(RAND_RUNTIME);
                    "crt::rand()".to_string()
                }
                Some("srand") => {
                    self.require(RAND_RUNTIME);
                    format!("crt::srand({})", self.expr(&args[1]))
                }
                Some("scanf") => stdio::scanf(self, &args[1..]),
                Some("fscanf") if args.get(1).and_then(stdio::stream) == Some("stdin") => {
                    stdio::scanf(self, &args[2..])
                }
                name => unsupported(&format!("call of {}", name.unwrap_or("a function pointer"))),
            },
            other => unsupported(other),
        }
    }

    /// `strtol` and `strtoll`, which are the same on x86-64 Linux, and
    /// `strtoul` and `strtoull`, with a null end pointer or the address of a
    /// `char *` variable.
    fn strtol(&mut self, name: &str, args: &[Value]) -> String {
        let end = match self.end_pointer("strtol", &args[1]) {
            Ok(end) => end,
            Err(err) => return err,
        };
        self.require(ERRNO_RUNTIME);
        self.require(ERANGE_RUNTIME);
        self.require(STRTOL_RUNTIME);
        let function = if name.starts_with("strtoul") {
            self.require(STRTOUL_RUNTIME);
            "strtoul"
        } else {
            self.require(STRTOL_SIGNED_RUNTIME);
            "strtol"
        };
        format!(
            "crt::{function}({}, {end}, {})",
            self.expr(&args[0]),
            self.expr(&args[2])
        )
    }

    /// The end pointer of `strtol` or `strtod`, null or the address of a
    /// `char *` variable, as the runtime takes it, or the unsupported error.
    fn end_pointer(&mut self, name: &str, end: &Value) -> Result<String, String> {
        if null_pointer(end) {
            return Ok("None".to_string());
        }
        let mut end = end;
        while kind(end) == "ParenExpr" {
            end = &inner(end)[0];
        }
        match inner(end).first() {
            Some(target)
                if kind(end) == "UnaryOperator"
                    && end["opcode"] == "&"
                    && c_type(target) == "char *"
                    && variable(target) =>
            {
                Ok(format!("Some(&mut {})", self.place(target)))
            }
            _ => Err(unsupported(&format!(
                "{name} with an end pointer other than &variable"
            ))),
        }
    }

    /// `fgets` from `stdin`, `memset`, `strcpy` and `strncpy`, which write
    /// the bytes of a local `char` array that their first argument decays
    /// from, borrowing it mutably during the call. Their result, the array,
    /// is not a `char *`: `fgets` returns whether it read, as `Some(())`.
    fn write_bytes(&mut self, name: &str, args: &[Value]) -> String {
        let Some(array) = self.buffer(&args[0]) else {
            return unsupported(&format!("{name} into anything but a local char array"));
        };
        let rest: Vec<String> = match name {
            "fgets" if args.get(2).and_then(stdio::stream) != Some("stdin") => {
                return unsupported("fgets from a stream other than stdin");
            }
            "fgets" => vec![self.expr(&args[1])],
            _ => args[1..].iter().map(|arg| self.expr(arg)).collect(),
        };
        let part = match name {
            "fgets" => stdio::FGETS_RUNTIME,
            "memset" => MEMSET_RUNTIME,
            "strcpy" => STRCPY_RUNTIME,
            _ => STRNCPY_RUNTIME,
        };
        self.require(part);
        format!("crt::{name}(&mut {array}[..], {})", rest.join(", "))
    }

    fn cast(&mut self, expr: &Value) -> String {
        let operand = &inner(expr)[0];
        match expr["castKind"].as_str().unwrap_or("") {
            // Reading a struct copies it.
            "LValueToRValue" if self.record(c_type(operand)).is_some() => {
                format!("{}.clone()", self.operand(operand))
            }
            "LValueToRValue" | "NoOp" | "FunctionToPointerDecay" => self.expr(operand),
            "NullToPointer" | "BitCast" if null_pointer(expr) && self.pointer(c_type(expr)) => {
                "None".to_string()
            }
            "ArrayToPointerDecay" => match stdio::string_array(operand) {
                Some(bytes) if std::str::from_utf8(&bytes).is_ok() => {
                    format!("Some({}.as_slice())", stdio::byte_string(&bytes))
                }
                // `%s` prints the bytes as Rust text.
                Some(_) => unsupported("string literal that is not UTF-8"),
                None if variable(operand) && char_array(c_type(operand)) => {
                    format!("Some(&{}[..])", self.expr(operand))
                }
                None => match self.elements(expr) {
                    Some(array) if variable(operand) => format!("Some(&{array}[..])"),
                    _ => unsupported("cast ArrayToPointerDecay"),
                },
            },
            // C converts a scalar to `bool` by testing it, as a condition
            // does. A constant, such as `<stdbool.h>`'s `true`, is a literal.
            "IntegralToBoolean" | "PointerToBoolean" => {
                let mut value = operand;
                while kind(value) == "ParenExpr" {
                    value = &inner(value)[0];
                }
                match (kind(value), value["value"].as_str()) {
                    ("IntegerLiteral", Some(v)) => (v != "0").to_string(),
                    _ => self.cond(operand),
                }
            }
            // `false` and `true` convert to 0 and 1.
            "IntegralCast" if is_bool(c_type(operand)) => match rust_type(c_type(expr)) {
                Some(to) => format!("{to}::from({})", self.expr(operand)),
                None => unsupported(&format!("cast to {}", c_type(expr))),
            },
            "IntegralCast" => match (rust_type(c_type(operand)), rust_type(c_type(expr))) {
                (Some(from), Some(to)) if from == to => self.expr(operand),
                (Some(_), Some(to)) => format!("{} as {to}", self.operand(operand)),
                _ => unsupported(&format!("cast to {}", c_type(expr))),
            },
            "FloatingToBoolean" => self.cond(operand),
            // Rust's `as` rounds to the nearest floating value, as C does.
            "IntegralToFloating" | "FloatingCast" => match float_type(c_type(expr)) {
                Some(to) if is_bool(c_type(operand)) => {
                    format!("{to}::from(u8::from({}))", self.expr(operand))
                }
                Some(to) => format!("{} as {to}", self.operand(operand)),
                None => unsupported(&format!("cast to {}", c_type(expr))),
            },
            // Both truncate towards zero. C leaves a value out of range
            // undefined, where Rust's `as` saturates.
            "FloatingToIntegral" => match rust_type(c_type(expr)) {
                Some(to) => format!("{} as {to}", self.operand(operand)),
                None => unsupported(&format!("cast to {}", c_type(expr))),
            },
            other => unsupported(&format!("cast {other}")),
        }
    }

    /// The right operand of the C operator `op` on `ty`, as `binary` takes
    /// it: the argument of a method needs no parentheses.
    fn right_operand(&mut self, op: &str, ty: &str, expr: &Value) -> String {
        if wraps(op, ty) {
            self.expr(expr)
        } else {
            self.operand(expr)
        }
    }

    /// An operand of an operator, in parentheses unless it is atomic, so the
    /// tree's grouping survives Rust's precedence rules.
    pub(crate) fn operand(&mut self, expr: &Value) -> String {
        let text = self.expr(expr);
        if atomic(expr) {
            text
        } else {
            format!("({text})")
        }
    }
}

fn atomic(expr: &Value) -> bool {
    match kind(expr) {
        "IntegerLiteral" | "FloatingLiteral" | "DeclRefExpr" | "CallExpr" => true,
        // A `char` is a byte converted `as i8`.
        "ArraySubscriptExpr" => c_type(expr) != "char",
        // A literal such as `4_u64`.
        "UnaryExprOrTypeTraitExpr" => true,
        // A negative one is printed with its sign.
        "CharacterLiteral" => expr["value"].as_u64().is_some_and(|v| v <= i32::MAX as u64),
        "ParenExpr" | "MemberExpr" => atomic(&inner(expr)[0]),
        "ImplicitCastExpr" => {
            matches!(expr["castKind"].as_str(), Some("LValueToRValue" | "NoOp"))
                && atomic(&inner(expr)[0])
        }
        _ => false,
    }
}

/// Whether `expr` names a variable, perhaps in parentheses.
fn variable(expr: &Value) -> bool {
    match kind(expr) {
        "ParenExpr" => variable(&inner(expr)[0]),
        "DeclRefExpr" => matches!(
            expr["referencedDecl"]["kind"].as_str(),
            Some("VarDecl" | "ParmVarDecl")
        ),
        _ => false,
    }
}

fn signed(c: &str) -> bool {
    rust_type(c).is_some_and(|ty| ty.starts_with('i'))
}

/// The binary operators that compute an integer from two integers.
const ARITHMETIC: &[&str] = &["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"];

/// Whether C wraps `op` on the type `ty`, as it does unsigned `+`, `-` and
/// `*`, where Rust's operator would check for overflow.
fn wraps(op: &str, ty: &str) -> bool {
    matches!(op, "+" | "-" | "*") && rust_type(ty).is_some() && !signed(ty)
}

/// The Rust form of the C operator `op` in `ARITHMETIC` on two translated
/// operands, where the C result has type `ty`. Clang has converted both
/// operands to `ty`, except the count of a shift, which Rust takes of any
/// integer type. Rust checks signed overflow, a zero divisor and a shift
/// count outside the type, which C leaves undefined.
fn binary(op: &str, ty: &str, left: &str, right: &str) -> String {
    if wraps(op, ty) {
        let method = match op {
            "+" => "wrapping_add",
            "-" => "wrapping_sub",
            _ => "wrapping_mul",
        };
        format!("{left}.{method}({right})")
    } else {
        format!("{left} {op} {right}")
    }
}

/// `value`, of the C type `ty`, converted to a bit-field of `width` bits if
/// it is one: C keeps the low bits, and GCC and Clang read those of a signed
/// type as a signed number of that width.
fn truncate(width: Option<u32>, ty: &str, value: &str) -> String {
    let Some((width, rust)) = width.zip(rust_type(ty)) else {
        return value.to_string();
    };
    let bits: u32 = rust[1..].parse().unwrap_or(0);
    if width >= bits {
        value.to_string()
    } else if signed(ty) {
        let shift = bits - width;
        format!("(({value}) << {shift}) >> {shift}")
    } else {
        format!("({value}) & {}", (1u64 << width) - 1)
    }
}

/// Sets `cell` to `new` of its value. The cell is found once, so an index
/// into an array is computed once, as C computes it.
fn update(cell: &str, new: impl FnOnce(&str) -> String) -> String {
    format!(
        "{{ let __cell = &{cell}; __cell.set({}) }}",
        new("__cell.get()")
    )
}

/// Whether `expr` is C's `errno`, which glibc's `<errno.h>` defines as
/// `(*__errno_location ())`.
fn errno(expr: &Value) -> bool {
    kind(expr) == "UnaryOperator"
        && expr["opcode"] == "*"
        && inner(expr).first().is_some_and(|call| {
            kind(call) == "CallExpr" && callee(call) == Some("__errno_location")
        })
}

/// The name of a directly called function.
pub(crate) fn callee(call: &Value) -> Option<&str> {
    let mut callee = inner(call).first()?;
    while matches!(kind(callee), "ImplicitCastExpr" | "ParenExpr") {
        callee = inner(callee).first()?;
    }
    if kind(callee) == "DeclRefExpr" && callee["referencedDecl"]["kind"] == "FunctionDecl" {
        callee["referencedDecl"]["name"].as_str()
    } else {
        None
    }
}

/// Records the variables that `node` assigns, increments or takes the
/// address of, other than structs, whose C type `structure` tells, as the
/// translation writes them only through their members' cells.
fn collect_mutated(node: &Value, mutated: &mut HashSet<String>, structure: &dyn Fn(&str) -> bool) {
    let writes = match kind(node) {
        "BinaryOperator" => node["opcode"] == "=",
        "CompoundAssignOperator" => true,
        "UnaryOperator" => matches!(node["opcode"].as_str(), Some("&" | "++" | "--")),
        _ => false,
    };
    if writes && let Some(mut target) = inner(node).first() {
        while matches!(kind(target), "ParenExpr" | "ImplicitCastExpr") {
            target = &inner(target)[0];
        }
        if let Some(id) = target["referencedDecl"]["id"].as_str()
            && !structure(c_type(target))
        {
            mutated.insert(id.to_string());
        }
    }
    for child in inner(node) {
        collect_mutated(child, mutated, structure);
    }
}

/// Records the integer variables whose address `node` takes, which the
/// translation holds in cells. An address passed straight to `scanf` is only
/// written through during the call, so its variable needs none.
fn collect_pointed_to(node: &Value, cells: &mut HashSet<String>) {
    if kind(node) == "CallExpr" && matches!(callee(node), Some("scanf" | "fscanf")) {
        for arg in inner(node) {
            if kind(arg) == "UnaryOperator" && arg["opcode"] == "&" {
                inner(arg)
                    .iter()
                    .flat_map(inner)
                    .for_each(|child| collect_pointed_to(child, cells));
            } else {
                collect_pointed_to(arg, cells);
            }
        }
        return;
    }
    if kind(node) == "UnaryOperator" && node["opcode"] == "&" {
        let mut target = &inner(node)[0];
        while kind(target) == "ParenExpr" {
            target = &inner(target)[0];
        }
        if variable(target)
            && cell_type(c_type(target)).is_some()
            && let Some(id) = target["referencedDecl"]["id"].as_str()
        {
            cells.insert(id.to_string());
        }
    }
    for child in inner(node) {
        collect_pointed_to(child, cells);
    }
}

/// The memory order of every access to a static local variable, which only
/// the program's one thread reads and writes.
const RELAXED: &str = "std::sync::atomic::Ordering::Relaxed";

/// The part of the `crt` module that builds `main`'s `argv`.
const ARGS_RUNTIME: &str = r#"    /// The program's arguments as C's `argv`: the bytes of each, through a
    /// zero that ends it, and then a null pointer.
    pub fn args() -> &'static [Option<&'static [u8]>] {
        use std::os::unix::ffi::OsStringExt;
        let args: Vec<Option<&'static [u8]>> = std::env::args_os()
            .map(|arg| {
                let mut bytes = arg.into_vec();
                bytes.push(0);
                let bytes: &'static [u8] = bytes.leak();
                Some(bytes)
            })
            .chain([None])
            .collect();
        args.leak()
    }
"#;

/// The part of the `crt` module that holds `errno`.
const ERRNO_RUNTIME: &str = r#"    /// C's `errno`, which the functions here set as glibc does.
    pub static ERRNO: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
"#;

/// The part of the `crt` module that names C's `ERANGE`, for the functions
/// that set `errno` to it.
const ERANGE_RUNTIME: &str = r#"    /// C's `ERANGE` on Linux.
    const ERANGE: i32 = 34;
"#;

/// The part of the `crt` module that names C's `EDOM`, for the functions
/// that set `errno` to it.
const EDOM_RUNTIME: &str = r#"    /// C's `EDOM` on Linux.
    const EDOM: i32 = 33;
"#;

/// The part of the `crt` module that `strtol`, `strtoll`, `strtoul` and
/// `strtoull` share. It sets `errno`, so the translation needs that part and
/// `ERANGE`'s too.
const STRTOL_RUNTIME: &str = r#"    /// Reads a number as glibc's `strtol` and `strtoul` do: skips white
    /// space, reads a sign and digits in `base`, or in the base their prefix
    /// gives for 0. Stores where the number ends, or the start of the string
    /// if there is none, through `end`. Returns the sign and the magnitude,
    /// or None for one beyond `u64`. C has no string at a null pointer.
    fn strto<'a>(
        s: Option<&'a [u8]>,
        end: Option<&mut Option<&'a [u8]>>,
        base: i32,
    ) -> (bool, Option<u64>) {
        let bytes = s.expect("null pointer as a C string");
        let digit = |i: usize| match bytes.get(i) {
            Some(&d @ b'0'..=b'9') => u32::from(d - b'0'),
            Some(&d @ b'a'..=b'z') => u32::from(d - b'a') + 10,
            Some(&d @ b'A'..=b'Z') => u32::from(d - b'A') + 10,
            _ => u32::MAX,
        };
        let mut i = 0;
        while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')) {
            i += 1;
        }
        let negative = match bytes.get(i) {
            Some(&sign @ (b'+' | b'-')) => {
                i += 1;
                sign == b'-'
            }
            _ => false,
        };
        let mut radix = u32::try_from(base).unwrap_or(1);
        // A `0x` without a hex digit after it is the number 0.
        if (radix == 0 || radix == 16)
            && bytes.get(i) == Some(&b'0')
            && matches!(bytes.get(i + 1), Some(b'x' | b'X'))
            && digit(i + 2) < 16
        {
            i += 2;
            radix = 16;
        } else if radix == 0 {
            radix = if bytes.get(i) == Some(&b'0') { 8 } else { 10 };
        }
        let start = i;
        let mut value = Some(0u64);
        if (2..=36).contains(&radix) {
            while digit(i) < radix {
                value = value
                    .and_then(|v| v.checked_mul(u64::from(radix)))
                    .and_then(|v| v.checked_add(u64::from(digit(i))));
                i += 1;
            }
        }
        let parsed = i > start;
        if let Some(end) = end {
            *end = if parsed { Some(&bytes[i..]) } else { s };
        }
        (negative && parsed, value)
    }
"#;

/// The part of the `crt` module that `strtol` and `strtoll` call.
const STRTOL_SIGNED_RUNTIME: &str = r#"    /// C's `strtol`, which saturates a value out of range and sets `errno`
    /// to `ERANGE` for it.
    pub fn strtol<'a>(
        s: Option<&'a [u8]>,
        end: Option<&mut Option<&'a [u8]>>,
        base: i32,
    ) -> i64 {
        let (negative, magnitude) = strto(s, end, base);
        let value = magnitude.and_then(|m| {
            if negative {
                0i64.checked_sub_unsigned(m)
            } else {
                i64::try_from(m).ok()
            }
        });
        value.unwrap_or_else(|| {
            ERRNO.store(ERANGE, std::sync::atomic::Ordering::Relaxed);
            if negative { i64::MIN } else { i64::MAX }
        })
    }
"#;

/// The part of the `crt` module that `strtoul` and `strtoull` call.
const STRTOUL_RUNTIME: &str = r#"    /// C's `strtoul`, which negates a value after a `-` in `u64` and
    /// gives `u64::MAX` and sets `errno` to `ERANGE` for one out of range.
    pub fn strtoul<'a>(
        s: Option<&'a [u8]>,
        end: Option<&mut Option<&'a [u8]>>,
        base: i32,
    ) -> u64 {
        match strto(s, end, base) {
            (negative, Some(m)) if negative => m.wrapping_neg(),
            (_, Some(m)) => m,
            (_, None) => {
                ERRNO.store(ERANGE, std::sync::atomic::Ordering::Relaxed);
                u64::MAX
            }
        }
    }
"#;

/// The part of the `crt` module that `rand` and `srand` call.
const RAND_RUNTIME: &str = r#"    /// The state of glibc's `random`, which its `rand` returns: 31 words, of
    /// which each call adds the one at `rear` to the one at `front`.
    struct Random {
        words: [u32; 31],
        front: usize,
        rear: usize,
    }

    impl Random {
        /// Seeds the words as glibc's `srandom` does, with a linear
        /// congruential generator, and discards the first 310 results.
        fn new(seed: u32) -> Random {
            let mut words = [0u32; 31];
            words[0] = if seed == 0 { 1 } else { seed };
            for i in 1..31 {
                // glibc reads the previous word as a signed 32-bit value.
                let word = i64::from(words[i - 1] as i32);
                let mut next = 16807 * (word % 127773) - 2836 * (word / 127773);
                if next < 0 {
                    next += 2147483647;
                }
                words[i] = next as u32;
            }
            let mut random = Random {
                words,
                front: 3,
                rear: 0,
            };
            for _ in 0..310 {
                random.next();
            }
            random
        }

        fn next(&mut self) -> i32 {
            self.words[self.front] = self.words[self.front].wrapping_add(self.words[self.rear]);
            let result = (self.words[self.front] >> 1) as i32;
            self.front = (self.front + 1) % 31;
            self.rear = (self.rear + 1) % 31;
            result
        }
    }

    static RANDOM: std::sync::Mutex<Option<Random>> = std::sync::Mutex::new(None);

    /// C's `srand` as glibc has it.
    pub fn srand(seed: u32) {
        *RANDOM.lock().unwrap() = Some(Random::new(seed));
    }

    /// C's `rand` as glibc has it, seeded with 1 until `srand` is called.
    pub fn rand() -> i32 {
        RANDOM
            .lock()
            .unwrap()
            .get_or_insert_with(|| Random::new(1))
            .next()
    }
"#;

/// The part of the `crt` module that `div`, `ldiv` and `lldiv` call.
const DIV_RUNTIME: &str = r#"    /// C's `div`, `ldiv` and `lldiv` as `(quot, rem)`: the quotient rounded
    /// towards zero, as Rust's `/` rounds it, and the remainder. C leaves a
    /// zero divisor and a quotient out of range undefined, and the division
    /// panics for them.
    pub fn div<T>(numer: T, denom: T) -> (T, T)
    where
        T: Copy + std::ops::Div<Output = T> + std::ops::Rem<Output = T>,
    {
        (numer / denom, numer % denom)
    }
"#;

/// The part of the `crt` module that `atoi` calls, with `strtol`'s parts.
const ATOI_RUNTIME: &str = r#"    /// C's `atoi`, which glibc defines as `strtol` in base 10 converted to
    /// `int`.
    pub fn atoi(s: Option<&[u8]>) -> i32 {
        strtol(s, None, 10) as i32
    }
"#;

/// The part of the `crt` module that `strlen` calls.
const STRLEN_RUNTIME: &str = r#"    /// C's `strlen`: the number of bytes before the first zero. A string
    /// without one reads past its array, which panics.
    pub fn strlen(s: Option<&[u8]>) -> u64 {
        let s = s.expect("null pointer as a C string");
        let mut n = 0;
        while s[n] != 0 {
            n += 1;
        }
        n as u64
    }
"#;

/// The part of the `crt` module that `memset` calls.
const MEMSET_RUNTIME: &str = r#"    /// C's `memset` on bytes: sets the first `n` to `c` converted to
    /// `unsigned char`. Writing past the array panics.
    pub fn memset(s: &mut [u8], c: i32, n: u64) {
        s[..n as usize].fill(c as u8);
    }
"#;

/// The part of the `crt` module that `strcpy` calls.
const STRCPY_RUNTIME: &str = r#"    /// C's `strcpy`: copies the bytes of `src` through its first zero.
    /// Reading or writing past an array panics.
    pub fn strcpy(dest: &mut [u8], src: Option<&[u8]>) {
        let src = src.expect("null pointer as a C string");
        let mut i = 0;
        loop {
            dest[i] = src[i];
            if src[i] == 0 {
                break;
            }
            i += 1;
        }
    }
"#;

/// The part of the `crt` module that `strncpy` calls.
const STRNCPY_RUNTIME: &str = r#"    /// C's `strncpy`: copies the bytes of `src` before its first zero, at
    /// most `n`, and fills the rest of the `n` with zeros. Reading or
    /// writing past an array panics.
    pub fn strncpy(dest: &mut [u8], src: Option<&[u8]>, n: u64) {
        let src = src.expect("null pointer as a C string");
        let mut ended = false;
        for i in 0..n as usize {
            ended = ended || src[i] == 0;
            dest[i] = if ended { 0 } else { src[i] };
        }
    }
"#;

/// The part of the `crt` module that `strtod` and `atof` call, with the
/// parts of `errno` and `ERANGE`.
const STRTOD_RUNTIME: &str = r#"    /// C's `strtod` as glibc has it: skips white space, reads a sign and a
    /// decimal number, or `inf`, `infinity` or `nan` with an optional
    /// `(...)` in any case, and stores where the number ends, or the start of
    /// the string if there is none, through `end`. Rust's parser rounds the
    /// number to the nearest `f64`, as glibc does. A result beyond `f64` is
    /// infinite, and one of nonzero digits too small to be normal is
    /// subnormal or zero; both set `errno` to `ERANGE`. C has no string at a
    /// null pointer, and a hexadecimal number panics, as Rust cannot read it.
    pub fn strtod<'a>(s: Option<&'a [u8]>, end: Option<&mut Option<&'a [u8]>>) -> f64 {
        let bytes = s.expect("null pointer as a C string");
        let word = |i: usize, word: &[u8]| {
            bytes
                .get(i..i + word.len())
                .is_some_and(|b| b.eq_ignore_ascii_case(word))
        };
        let digits = |mut i: usize, hex: bool| {
            while bytes.get(i).is_some_and(|b| {
                if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() }
            }) {
                i += 1;
            }
            i
        };
        let mut i = 0;
        while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')) {
            i += 1;
        }
        let negative = match bytes.get(i) {
            Some(&sign @ (b'+' | b'-')) => {
                i += 1;
                sign == b'-'
            }
            _ => false,
        };
        let (value, used) = if word(i, b"infinity") {
            (f64::INFINITY, i + 8)
        } else if word(i, b"inf") {
            (f64::INFINITY, i + 3)
        } else if word(i, b"nan") {
            let mut used = i + 3;
            let mut close = used + 1;
            while bytes.get(close).is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_') {
                close += 1;
            }
            if bytes.get(used) == Some(&b'(') && bytes.get(close) == Some(&b')') {
                used = close + 1;
            }
            (f64::NAN, used)
        } else {
            if word(i, b"0x") && {
                let point = usize::from(bytes.get(i + 2) == Some(&b'.'));
                digits(i + 2 + point, true) > i + 2 + point
            } {
                panic!("hexadecimal floating-point number in strtod");
            }
            let whole = digits(i, false);
            let mut used = whole;
            if bytes.get(used) == Some(&b'.') {
                used = digits(used + 1, false);
            }
            let mantissa = &bytes[i..used];
            if !mantissa.iter().any(u8::is_ascii_digit) {
                if let Some(end) = end {
                    *end = s;
                }
                return 0.0;
            }
            if matches!(bytes.get(used), Some(b'e' | b'E')) {
                let sign = usize::from(matches!(bytes.get(used + 1), Some(b'+' | b'-')));
                let exponent = digits(used + 1 + sign, false);
                if exponent > used + 1 + sign {
                    used = exponent;
                }
            }
            let text = std::str::from_utf8(&bytes[i..used]).unwrap();
            let value: f64 = text.parse().unwrap();
            let nonzero = mantissa.iter().any(|&b| (b'1'..=b'9').contains(&b));
            if value.is_infinite() || (nonzero && (value == 0.0 || value.is_subnormal())) {
                ERRNO.store(ERANGE, std::sync::atomic::Ordering::Relaxed);
            }
            (value, used)
        };
        if let Some(end) = end {
            *end = Some(&bytes[used..]);
        }
        if negative { -value } else { value }
    }
"#;

/// The part of the `crt` module that `sqrt` calls, with the parts of `errno`
/// and `EDOM`.
const SQRT_RUNTIME: &str = r#"    /// C's `sqrt`, which glibc gives a negative number not a number for and
    /// sets `errno` to `EDOM`.
    pub fn sqrt(x: f64) -> f64 {
        if x < 0.0 {
            ERRNO.store(EDOM, std::sync::atomic::Ordering::Relaxed);
        }
        x.sqrt()
    }
"#;

/// The part of the `crt` module that `pow` calls, with the parts of `errno`,
/// `EDOM` and `ERANGE`.
const POW_RUNTIME: &str = r#"    /// C's `pow` from glibc, which Rust's `powf` calls, setting `errno` as
    /// glibc does: to `EDOM` for a result that is not a number of operands
    /// that are, and to `ERANGE` for an infinite result of finite operands
    /// or a zero one of a finite nonzero base and a finite exponent.
    pub fn pow(x: f64, y: f64) -> f64 {
        let result = x.powf(y);
        if result.is_nan() && !x.is_nan() && !y.is_nan() {
            ERRNO.store(EDOM, std::sync::atomic::Ordering::Relaxed);
        } else if x.is_finite()
            && y.is_finite()
            && (result.is_infinite() || (result == 0.0 && x != 0.0))
        {
            ERRNO.store(ERANGE, std::sync::atomic::Ordering::Relaxed);
        }
        result
    }
"#;
