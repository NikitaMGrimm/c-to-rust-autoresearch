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
    let ty = &node["type"];
    let ty = ty["desugaredQualType"]
        .as_str()
        .or(ty["qualType"].as_str())
        .unwrap_or("");
    ty.trim_start_matches("const ")
        .trim_start_matches("volatile ")
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

/// The Rust type of a C type that the translation can hold in a variable: an
/// integer, or a `char *` into a string literal, which the program can only
/// read. Such a pointer is `None` for null and otherwise the literal's bytes
/// from where it points, through the zero that ends the array.
pub(crate) fn value_type(c: &str) -> Option<&'static str> {
    match c {
        "char *" => Some("Option<&'static [u8]>"),
        c => rust_type(c),
    }
}

/// The value of an uninitialised variable of a C type that `value_type`
/// translates, as C gives the program no value to read.
pub(crate) fn zero(c: &str) -> &'static str {
    if c == "char *" { "None" } else { "0" }
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
    if translator.functions.contains("main") {
        // Rust's own `main` calls the C one and exits with its result.
        out.push_str(&format!(
            "fn main() {{\n    std::process::exit({}());\n}}\n",
            ident("main")
        ));
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

        let mut out = format!("fn {}(", ident(name));
        if name == "main" && !params.is_empty() {
            out.push_str(&format!(
                ") {{\n    {};\n}}\n",
                unsupported("main with parameters")
            ));
            return out;
        }
        let params: Vec<String> = params.iter().map(|p| self.binding(p)).collect();
        out.push_str(&params.join(", "));
        out.push(')');
        match ret {
            "void" => {}
            ret => match value_type(ret) {
                Some(ty) => out.push_str(&format!(" -> {ty}")),
                None => out.push_str(&format!(" -> {}", unsupported(&format!("type {ret}")))),
            },
        }
        out.push_str(" {\n");
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
        let ty = value_type(ty).map_or_else(|| unsupported(&format!("type {ty}")), String::from);
        format!(
            "{}{}: {ty}",
            self.mut_(var),
            ident(var["name"].as_str().unwrap_or(""))
        )
    }

    fn mut_(&self, var: &Value) -> &'static str {
        if var["id"]
            .as_str()
            .is_some_and(|id| self.mutated.contains(id))
        {
            "mut "
        } else {
            ""
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
            Some("static") => return format!("{};", unsupported("static local variable")),
            // It names an object defined elsewhere, not a fresh zero.
            Some("extern") => return format!("{};", unsupported("extern local variable")),
            _ => {}
        }
        // An uninitialised C variable has no value to read, so any value will do.
        let init = match inner(var).first() {
            Some(init) if var["init"].is_string() => self.expr(init),
            _ => zero(c_type(var)).to_string(),
        };
        format!("let {} = {init};", self.binding(var))
    }

    /// An expression whose value C discards, where assignments and output
    /// calls have their Rust statement forms.
    pub(crate) fn expr_stmt(&mut self, expr: &Value) -> String {
        let args = inner(expr);
        match (kind(expr), expr["opcode"].as_str()) {
            ("BinaryOperator", Some("=")) => {
                format!("{} = {}", self.place(&args[0]), self.expr(&args[1]))
            }
            ("CompoundAssignOperator", Some(op @ ("+=" | "-=" | "*=" | "/=" | "%="))) => {
                let ty = c_type(&args[0]);
                if ty != expr["computeLHSType"]["qualType"] || !signed(ty) {
                    return unsupported(&format!("{op} on {ty}"));
                }
                format!("{} {op} {}", self.place(&args[0]), self.expr(&args[1]))
            }
            ("UnaryOperator", Some(op @ ("++" | "--"))) => {
                let place = self.place(&args[0]);
                let (assign, method) = if op == "++" {
                    ("+=", "wrapping_add")
                } else {
                    ("-=", "wrapping_sub")
                };
                match c_type(&args[0]) {
                    // Overflow is undefined for these, so Rust may check it.
                    "int" | "long" | "long long" => format!("{place} {assign} 1"),
                    // Unsigned types wrap, and narrower ones convert back
                    // from `int`, which wraps on x86-64 too.
                    ty if rust_type(ty).is_some() => format!("{place} = {place}.{method}(1)"),
                    ty => unsupported(&format!("{op} on {ty}")),
                }
            }
            ("CallExpr", _) => match callee(expr) {
                Some("printf") => stdio::printf(self, &args[1..]),
                _ => self.expr(expr),
            },
            ("ParenExpr", _) => self.expr_stmt(&args[0]),
            _ => self.expr(expr),
        }
    }

    /// An lvalue in a position Rust writes to.
    pub(crate) fn place(&mut self, expr: &Value) -> String {
        match kind(expr) {
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
            "DeclRefExpr" => ident(expr["referencedDecl"]["name"].as_str().unwrap_or("")),
            "ParenExpr" => self.expr(&args[0]),
            "ImplicitCastExpr" | "CStyleCastExpr" => self.cast(expr),
            _ if control::boolean(expr) => format!("i32::from({})", self.cond(expr)),
            "UnaryOperator" => match expr["opcode"].as_str() {
                Some("-") if signed(c_type(expr)) => format!("-{}", self.operand(&args[0])),
                Some("+") => self.expr(&args[0]),
                op => unsupported(&format!("unary {}", op.unwrap_or("?"))),
            },
            "BinaryOperator" => match expr["opcode"].as_str() {
                Some(op @ ("+" | "-" | "*" | "/" | "%")) if signed(c_type(expr)) => {
                    format!("{} {op} {}", self.operand(&args[0]), self.operand(&args[1]))
                }
                op => unsupported(&format!("binary {} on {}", op.unwrap_or("?"), c_type(expr))),
            },
            "CallExpr" => match callee(expr) {
                Some(name) if self.functions.contains(name) => {
                    let args: Vec<String> = args[1..].iter().map(|a| self.expr(a)).collect();
                    format!("{}({})", ident(name), args.join(", "))
                }
                Some("scanf") => stdio::scanf(self, &args[1..]),
                Some("fscanf") if args.get(1).is_some_and(stdio::is_stdin) => {
                    stdio::scanf(self, &args[2..])
                }
                name => unsupported(&format!("call of {}", name.unwrap_or("a function pointer"))),
            },
            other => unsupported(other),
        }
    }

    fn cast(&mut self, expr: &Value) -> String {
        let operand = &inner(expr)[0];
        match expr["castKind"].as_str().unwrap_or("") {
            "LValueToRValue" | "NoOp" | "FunctionToPointerDecay" => self.expr(operand),
            "NullToPointer" | "BitCast" if null_pointer(expr) && c_type(expr) == "char *" => {
                "None".to_string()
            }
            "ArrayToPointerDecay" => match stdio::string_array(operand) {
                Some(bytes) if std::str::from_utf8(&bytes).is_ok() => {
                    format!("Some({}.as_slice())", stdio::byte_string(&bytes))
                }
                // `%s` prints the bytes as Rust text.
                Some(_) => unsupported("string literal that is not UTF-8"),
                None => unsupported("cast ArrayToPointerDecay"),
            },
            "IntegralCast" => match (rust_type(c_type(operand)), rust_type(c_type(expr))) {
                (Some(from), Some(to)) if from == to => self.expr(operand),
                (Some(_), Some(to)) => format!("{} as {to}", self.operand(operand)),
                _ => unsupported(&format!("cast to {}", c_type(expr))),
            },
            other => unsupported(&format!("cast {other}")),
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
        "IntegerLiteral" | "DeclRefExpr" | "CallExpr" => true,
        // A negative one is printed with its sign.
        "CharacterLiteral" => expr["value"].as_u64().is_some_and(|v| v <= i32::MAX as u64),
        "ParenExpr" => atomic(&inner(expr)[0]),
        "ImplicitCastExpr" => {
            matches!(expr["castKind"].as_str(), Some("LValueToRValue" | "NoOp"))
                && atomic(&inner(expr)[0])
        }
        _ => false,
    }
}

fn signed(c: &str) -> bool {
    rust_type(c).is_some_and(|ty| ty.starts_with('i'))
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
