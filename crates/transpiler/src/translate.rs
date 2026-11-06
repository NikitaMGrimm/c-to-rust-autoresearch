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

/// Whether `c` is C's boolean type, which `<stdbool.h>` spells `bool`.
pub(crate) fn is_bool(c: &str) -> bool {
    matches!(c, "bool" | "_Bool")
}

/// The Rust type of a C type that the translation can hold in a variable: an
/// integer, a `bool`, a `char *` into a string literal or a program
/// argument, a `char **` into `main`'s `argv`, a pointer to an integer
/// variable or array element, or the result of `div`. A pointer is `None`
/// for null. A `char *` is otherwise the string's bytes from where it
/// points, through the zero that ends the array, and a pointer to an integer
/// is the cells from the one it points to through the end of its array, a
/// variable being an array of one. A `char **` is the pointers from where it
/// points through the null that ends `argv`.
pub(crate) fn value_type(c: &str) -> Option<String> {
    match c {
        c if is_bool(c) => Some("bool".to_string()),
        "char *" => Some("Option<&'static [u8]>".to_string()),
        "char **" => Some("&'static [Option<&'static [u8]>]".to_string()),
        c => match c.strip_suffix(" *") {
            Some(pointee) => cell_type(pointee).map(|ty| format!("Option<&[{CELL}<{ty}>]>")),
            None => rust_type(c)
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

/// The Rust type of the members of `div_t`, `ldiv_t` or `lldiv_t`, which
/// the translation holds as the pair `(quot, rem)`.
fn quotient_type(c: &str) -> Option<&'static str> {
    match c {
        "div_t" => Some("i32"),
        "ldiv_t" | "lldiv_t" => Some("i64"),
        _ => None,
    }
}

/// Whether `value_type` translates the C type `c` as a pointer.
pub(crate) fn pointer(c: &str) -> bool {
    c.ends_with(" *") && value_type(c).is_some()
}

/// The value of an uninitialised variable of a C type that `value_type`
/// translates, as C gives the program no value to read.
pub(crate) fn zero(c: &str) -> &'static str {
    if pointer(c) {
        "None"
    } else if is_bool(c) {
        "false"
    } else if c == "char **" {
        "&[]"
    } else if quotient_type(c).is_some() {
        "(0, 0)"
    } else {
        "0"
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

pub(crate) struct Translator {
    /// Names of the functions the program defines.
    functions: HashSet<String>,
    /// Ids of the variables that are assigned or whose address is taken.
    mutated: HashSet<String>,
    /// Ids of the integer variables held in a cell, as the program points
    /// to them.
    cells: HashSet<String>,
    /// The Rust statics of the static local variables, by id.
    statics: HashMap<String, String>,
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
        mutated: HashSet::new(),
        cells: HashSet::new(),
        statics: HashMap::new(),
        runtime: Vec::new(),
        scopes: Vec::new(),
        loops: Vec::new(),
        labels: HashMap::new(),
        fresh: 0,
    };
    let mut out = String::new();
    for decl in inner(ast) {
        // Declarations from headers have no initialiser and are `extern`.
        if kind(decl) == "VarDecl" && decl["storageClass"] != "extern" {
            out.push_str(&unsupported("global variable"));
            out.push_str(";\n\n");
        }
    }
    for function in &definitions {
        out.push_str(&translator.function(function));
        out.push('\n');
    }
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
        self.mutated.clear();
        collect_mutated(body, &mut self.mutated);
        self.cells.clear();
        collect_pointed_to(body, &mut self.cells);

        let mut out = format!("fn {}(", ident(name));
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
        let params: Vec<String> = params.iter().map(|p| self.binding(p)).collect();
        out.push_str(&params.join(", "));
        out.push(')');
        match ret {
            "void" => {}
            // Rust has no lifetime for the variable such a pointer borrows.
            ret if pointer(ret) && ret != "char *" => {
                out.push_str(&format!(
                    " -> {}",
                    unsupported(&format!("return type {ret}"))
                ));
            }
            ret => match value_type(ret) {
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
        let ty = c_type(var);
        let ty = value_type(ty).unwrap_or_else(|| unsupported(&format!("type {ty}")));
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
            _ => zero(c_type(var)).to_string(),
        };
        self.declaration(var, &init)
    }

    /// A local array of integers as cells, which pointers into it share: a
    /// borrowed Rust array for a fixed length and a `Vec` for a variable
    /// one. Clang gives a variable length only as the text of its
    /// expression, so it must be a variable's name. Elements without an
    /// initialiser are 0, as C gives no value to read.
    fn array(&mut self, var: &Value, element: &str, length: &str) -> String {
        let name = ident(var["name"].as_str().unwrap_or(""));
        let Some(ty) = cell_type(element) else {
            return format!("{};", unsupported(&format!("type {}", c_type(var))));
        };
        let init = inner(var).first().filter(|_| var["init"].is_string());
        if !length.is_empty() && length.bytes().all(|b| b.is_ascii_digit()) {
            let array = format!("[const {{ {CELL}::new(0) }}; {length}]");
            let value = match init {
                None => array,
                Some(list) if kind(list) == "InitListExpr" => {
                    // With a filler for the elements it leaves out, Clang
                    // lists the filler and then the elements.
                    let elements = match list["array_filler"].as_array() {
                        Some(filler) => &filler[1..],
                        None => inner(list),
                    };
                    let mut sets = String::new();
                    for (i, element) in elements.iter().enumerate() {
                        if kind(element) != "ImplicitValueInitExpr" {
                            sets.push_str(&format!(" __array[{i}].set({});", self.expr(element)));
                        }
                    }
                    if sets.is_empty() {
                        array
                    } else {
                        format!("{{ let __array = {array};{sets} __array }}")
                    }
                }
                Some(_) => return format!("{};", unsupported("array initialiser")),
            };
            // As a slice, so a constant index out of bounds panics when it
            // runs, as an array's would fail to compile even where it never
            // runs.
            return format!("let {name}: &[{CELL}<{ty}>] = &{value};");
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
            "let {name}: Vec<{CELL}<{ty}>> = vec![{CELL}::new(0); {} as usize];",
            ident(length)
        )
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
            _ => zero(ty).to_string(),
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
                update(&cell, |old| binary(op, ty, old, &value))
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
                if ty != expr["computeLHSType"]["qualType"]
                    || rust_type(ty).is_none()
                    || !ARITHMETIC.contains(&op)
                {
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
                    return update(&cell, step);
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
        match self.cell(target) {
            Some(cell) => format!("{cell}.set({value})"),
            None => format!("{} = {value}", self.place(target)),
        }
    }

    /// The Rust cell of a C lvalue that the translation holds in one: an
    /// integer variable that the program points to, an element of an
    /// integer array or `*p` of a pointer to an integer.
    fn cell(&mut self, expr: &Value) -> Option<String> {
        let args = inner(expr);
        match kind(expr) {
            "ParenExpr" => self.cell(&args[0]),
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

    /// The cells that a pointer to an integer points to, from the first: the
    /// array that decays to it, or the pointer's slice, as C has no object
    /// at null.
    fn elements(&mut self, pointer: &Value) -> Option<String> {
        cell_type(c_type(pointer).strip_suffix(" *")?)?;
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
                    None => ident(expr["referencedDecl"]["name"].as_str().unwrap_or("")),
                },
            },
            _ if errno(expr) => {
                self.require(ERRNO_RUNTIME);
                format!("crt::ERRNO.load({RELAXED})")
            }
            // The size of an integer or a pointer on x86-64 Linux.
            "UnaryExprOrTypeTraitExpr" if expr["name"] == "sizeof" => {
                let of = match args.first() {
                    Some(operand) => c_type(operand),
                    None => qual_type(&expr["argType"]),
                };
                let size = |of: &str| match rust_type(of) {
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
            // An element of `argv`.
            "ArraySubscriptExpr"
                if c_type(&args[0]) == "char **" && rust_type(c_type(&args[1])).is_some() =>
            {
                format!("{}[{}]", self.operand(&args[0]), self.index(&args[1]))
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
            _ if control::boolean(expr) => format!("i32::from({})", self.cond(expr)),
            "UnaryOperator" => match expr["opcode"].as_str() {
                Some("-") if signed(c_type(expr)) => format!("-{}", self.operand(&args[0])),
                // C negates an unsigned value modulo its range.
                Some("-") if rust_type(c_type(expr)).is_some() => {
                    format!("{}.wrapping_neg()", self.operand(&args[0]))
                }
                Some("~") if rust_type(c_type(expr)).is_some() => {
                    format!("!{}", self.operand(&args[0]))
                }
                Some("+") => self.expr(&args[0]),
                // The address of an integer variable, the one cell it holds,
                // or of an array element, the cells from it to the array's
                // end, which may be none past its last element.
                Some("&") if c_type(expr) != "char *" && pointer(c_type(expr)) => {
                    let mut target = &args[0];
                    while kind(target) == "ParenExpr" {
                        target = &inner(target)[0];
                    }
                    match (kind(target), self.cell(target)) {
                        ("DeclRefExpr", Some(cell)) => {
                            format!("Some(std::slice::from_ref(&{cell}))")
                        }
                        ("ArraySubscriptExpr", Some(_)) => {
                            let target = inner(target);
                            let elements = self.elements(&target[0]).unwrap_or_default();
                            format!("Some(&{elements}[{}..])", self.index(&target[1]))
                        }
                        _ => unsupported("unary &"),
                    }
                }
                Some("*") if c_type(&args[0]) != "char *" && pointer(c_type(&args[0])) => {
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
        let end = if null_pointer(&args[1]) {
            "None".to_string()
        } else {
            let mut end = &args[1];
            while kind(end) == "ParenExpr" {
                end = &inner(end)[0];
            }
            let target = inner(end).first();
            match target {
                Some(target)
                    if kind(end) == "UnaryOperator"
                        && end["opcode"] == "&"
                        && c_type(target) == "char *"
                        && variable(target) =>
                {
                    format!("Some(&mut {})", self.place(target))
                }
                _ => return unsupported("strtol with an end pointer other than &variable"),
            }
        };
        self.require(ERRNO_RUNTIME);
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

    fn cast(&mut self, expr: &Value) -> String {
        let operand = &inner(expr)[0];
        match expr["castKind"].as_str().unwrap_or("") {
            "LValueToRValue" | "NoOp" | "FunctionToPointerDecay" => self.expr(operand),
            "NullToPointer" | "BitCast" if null_pointer(expr) && pointer(c_type(expr)) => {
                "None".to_string()
            }
            "ArrayToPointerDecay" => match stdio::string_array(operand) {
                Some(bytes) if std::str::from_utf8(&bytes).is_ok() => {
                    format!("Some({}.as_slice())", stdio::byte_string(&bytes))
                }
                // `%s` prints the bytes as Rust text.
                Some(_) => unsupported("string literal that is not UTF-8"),
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
        "IntegerLiteral" | "DeclRefExpr" | "CallExpr" | "ArraySubscriptExpr" => true,
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
/// address of.
fn collect_mutated(node: &Value, mutated: &mut HashSet<String>) {
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
        if let Some(id) = target["referencedDecl"]["id"].as_str() {
            mutated.insert(id.to_string());
        }
    }
    for child in inner(node) {
        collect_mutated(child, mutated);
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

/// The part of the `crt` module that `strtol`, `strtoll`, `strtoul` and
/// `strtoull` share. It sets `errno`, so the translation needs that part too.
const STRTOL_RUNTIME: &str = r#"    /// Reads a number as glibc's `strtol` and `strtoul` do: skips white
    /// space, reads a sign and digits in `base`, or in the base their prefix
    /// gives for 0. Stores where the number ends, or the start of the string
    /// if there is none, through `end`. Returns the sign and the magnitude,
    /// or None for one beyond `u64`. C has no string at a null pointer.
    fn strto(
        s: Option<&'static [u8]>,
        end: Option<&mut Option<&'static [u8]>>,
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

    /// C's `ERANGE` on Linux.
    const ERANGE: i32 = 34;
"#;

/// The part of the `crt` module that `strtol` and `strtoll` call.
const STRTOL_SIGNED_RUNTIME: &str = r#"    /// C's `strtol`, which saturates a value out of range and sets `errno`
    /// to `ERANGE` for it.
    pub fn strtol(
        s: Option<&'static [u8]>,
        end: Option<&mut Option<&'static [u8]>>,
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
    pub fn strtoul(
        s: Option<&'static [u8]>,
        end: Option<&mut Option<&'static [u8]>>,
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
