//! C's control flow: conditions, `if`, the three loops, `break`, `continue`
//! and `goto`. A block with labels becomes a loop around a `match` on the
//! segment to run next, so a `goto` to one of its labels sets the segment and
//! continues that loop.

use crate::translate::{
    Translator, c_type, ident, inner, kind, null_pointer, pointer, rust_type, unsupported,
    value_type, zero,
};
use serde_json::Value;

/// A Rust loop or labelled block around the statement being translated.
pub(crate) struct Scope {
    label: String,
    is_loop: bool,
    /// Whether a jump names the label, so the scope must carry it.
    used: bool,
}

/// Where a jump goes: the scope it leaves, and whether it continues it.
#[derive(Clone, Copy)]
pub(crate) struct Jump {
    scope: usize,
    continues: bool,
}

/// Where a C loop's `break` and `continue` go.
pub(crate) struct CLoop {
    break_: Jump,
    continue_: Jump,
}

/// A label in a block being translated: the scope of the block's loop, the
/// variable naming its segment and the label's segment.
pub(crate) struct Label {
    scope: usize,
    var: String,
    segment: usize,
}

fn comparison(op: &str) -> bool {
    matches!(op, "<" | ">" | "<=" | ">=" | "==" | "!=")
}

fn logical(op: &str) -> bool {
    matches!(op, "&&" | "||")
}

fn strip_parens(mut expr: &Value) -> &Value {
    while kind(expr) == "ParenExpr" {
        expr = &inner(expr)[0];
    }
    expr
}

/// Whether C gives `expr` the value 0 or 1 from a test, so that Rust has it
/// as a `bool`.
pub(crate) fn boolean(expr: &Value) -> bool {
    let expr = strip_parens(expr);
    match (kind(expr), expr["opcode"].as_str()) {
        ("BinaryOperator", Some(op)) => comparison(op) || logical(op),
        ("UnaryOperator", Some("!")) => true,
        _ => false,
    }
}

/// A nonzero integer constant as a condition, as in `while (1)`.
fn always(cond: &Value) -> bool {
    let cond = strip_parens(cond);
    kind(cond) == "IntegerLiteral" && cond["value"].as_str().is_some_and(|v| v != "0")
}

/// Whether control never reaches the end of `stmt`.
fn jumps(stmt: &Value) -> bool {
    matches!(
        kind(stmt),
        "GotoStmt" | "ReturnStmt" | "BreakStmt" | "ContinueStmt"
    )
}

/// Adds one level of indentation to translated lines.
fn indent(lines: &str) -> String {
    lines.lines().map(|line| format!("    {line}\n")).collect()
}

impl Translator {
    /// A C expression in a position that tests it, as a Rust `bool`.
    pub(crate) fn cond(&mut self, expr: &Value) -> String {
        let expr = strip_parens(expr);
        let args = inner(expr);
        match (kind(expr), expr["opcode"].as_str()) {
            // A pointer equals only null, whatever the integer constant.
            ("BinaryOperator", Some(op @ ("==" | "!=")))
                if null_pointer(&args[0]) || null_pointer(&args[1]) =>
            {
                let pointer = if null_pointer(&args[1]) {
                    &args[0]
                } else {
                    &args[1]
                };
                self.nonzero(pointer, op)
            }
            ("BinaryOperator", Some(op)) if comparison(op) => {
                match (rust_type(c_type(&args[0])), rust_type(c_type(&args[1]))) {
                    (Some(a), Some(b)) if a == b => {
                        format!("{} {op} {}", self.operand(&args[0]), self.operand(&args[1]))
                    }
                    _ => unsupported(&format!("comparison of {}", c_type(&args[0]))),
                }
            }
            ("BinaryOperator", Some(op)) if logical(op) => {
                let left = self.cond_operand(&args[0], op, true);
                let right = self.cond_operand(&args[1], op, false);
                format!("{left} {op} {right}")
            }
            ("UnaryOperator", Some("!")) if boolean(&args[0]) => {
                format!("!({})", self.cond(&args[0]))
            }
            ("UnaryOperator", Some("!")) => self.nonzero(&args[0], "=="),
            _ => self.nonzero(expr, "!="),
        }
    }

    /// An operand of `&&` or `||`, in parentheses where Rust would group it
    /// differently from the tree.
    fn cond_operand(&mut self, expr: &Value, op: &str, left: bool) -> String {
        let text = self.cond(expr);
        let expr = strip_parens(expr);
        match expr["opcode"].as_str() {
            Some(inner_op) if kind(expr) == "BinaryOperator" && logical(inner_op) => {
                if left && inner_op == op {
                    text
                } else {
                    format!("({text})")
                }
            }
            _ => text,
        }
    }

    /// Compares an integer to zero, or a pointer to null, with `==` or `!=`.
    fn nonzero(&mut self, expr: &Value, op: &str) -> String {
        if pointer(c_type(expr)) {
            let test = if op == "==" { "is_none" } else { "is_some" };
            return format!("{}.{test}()", self.operand(expr));
        }
        if rust_type(c_type(expr)).is_none() {
            return unsupported(&format!("condition of type {}", c_type(expr)));
        }
        format!("{} {op} 0", self.operand(expr))
    }

    /// Translates a loop or block body. A compound statement shares the
    /// Rust block, as its scope is the same.
    pub(crate) fn body(&mut self, stmt: &Value, depth: usize, out: &mut String) {
        if kind(stmt) == "CompoundStmt" {
            self.items(inner(stmt), depth, out);
        } else {
            self.stmt(stmt, depth, out);
        }
    }

    /// The statements of a block, which needs a segment loop if it has labels.
    pub(crate) fn items(&mut self, stmts: &[Value], depth: usize, out: &mut String) {
        if stmts.iter().any(|s| kind(s) == "LabelStmt") {
            self.segments(stmts, depth, out);
        } else {
            for stmt in stmts {
                self.stmt(stmt, depth, out);
            }
        }
    }

    /// Opens a Rust loop or labelled block that jumps can leave.
    fn push_scope(&mut self, is_loop: bool) -> usize {
        self.scopes.push(Scope {
            label: format!("'l{}", self.fresh),
            is_loop,
            used: false,
        });
        self.fresh += 1;
        self.scopes.len() - 1
    }

    /// Closes the innermost scope and returns its label if a jump uses it.
    fn pop_scope(&mut self) -> String {
        match self.scopes.pop() {
            Some(scope) if scope.used => format!("{}: ", scope.label),
            _ => String::new(),
        }
    }

    /// `break` or `continue`, labelled unless it leaves the innermost loop.
    fn jump(&mut self, jump: Jump) -> String {
        let word = if jump.continues { "continue" } else { "break" };
        let innermost = jump.scope + 1 == self.scopes.len();
        let scope = &mut self.scopes[jump.scope];
        if innermost && scope.is_loop {
            word.to_string()
        } else {
            scope.used = true;
            format!("{word} {}", scope.label)
        }
    }

    /// Translates control flow statements, or returns false for another kind.
    pub(crate) fn control(&mut self, stmt: &Value, depth: usize, out: &mut String) -> bool {
        let pad = "    ".repeat(depth);
        let args = inner(stmt);
        match kind(stmt) {
            "IfStmt" => {
                let mut stmt = stmt;
                let mut keyword = "if";
                loop {
                    let args = inner(stmt);
                    let cond = self.cond(&args[0]);
                    out.push_str(&format!("{pad}{keyword} {cond} {{\n"));
                    self.body(&args[1], depth + 1, out);
                    match args.get(2) {
                        Some(next) if kind(next) == "IfStmt" => {
                            stmt = next;
                            keyword = "} else if";
                        }
                        Some(other) => {
                            out.push_str(&format!("{pad}}} else {{\n"));
                            self.body(other, depth + 1, out);
                            break;
                        }
                        None => break,
                    }
                }
                out.push_str(&format!("{pad}}}\n"));
            }
            "WhileStmt" if args.len() == 2 => {
                let header = if always(&args[0]) {
                    "loop".to_string()
                } else {
                    format!("while {}", self.cond(&args[0]))
                };
                let scope = self.push_scope(true);
                self.loops.push(CLoop {
                    break_: Jump {
                        scope,
                        continues: false,
                    },
                    continue_: Jump {
                        scope,
                        continues: true,
                    },
                });
                let mut body = String::new();
                self.body(&args[1], depth + 1, &mut body);
                self.loops.pop();
                let label = self.pop_scope();
                out.push_str(&format!("{pad}{label}{header} {{\n{body}{pad}}}\n"));
            }
            "DoStmt" => {
                let test = if always(&args[1]) {
                    String::new()
                } else {
                    let cond = self.cond(&args[1]);
                    format!("{pad}    if !({cond}) {{\n{pad}        break;\n{pad}    }}\n")
                };
                self.stepped_loop("loop", &args[0], &test, depth, out);
            }
            "ForStmt" => {
                let (init, cond, step, body) = (&args[0], &args[2], &args[3], &args[4]);
                let scoped = kind(init) == "DeclStmt";
                let depth = if scoped {
                    out.push_str(&format!("{pad}{{\n"));
                    depth + 1
                } else {
                    depth
                };
                if !kind(init).is_empty() {
                    self.stmt(init, depth, out);
                }
                let header = if kind(cond).is_empty() || always(cond) {
                    "loop".to_string()
                } else {
                    format!("while {}", self.cond(cond))
                };
                let step = if kind(step).is_empty() {
                    String::new()
                } else {
                    format!("{}    {};\n", "    ".repeat(depth), self.expr_stmt(step))
                };
                self.stepped_loop(&header, body, &step, depth, out);
                if scoped {
                    out.push_str(&format!("{pad}}}\n"));
                }
            }
            "BreakStmt" | "ContinueStmt" => {
                let breaks = kind(stmt) == "BreakStmt";
                let target = self.loops.last().map(|c_loop| {
                    if breaks {
                        c_loop.break_
                    } else {
                        c_loop.continue_
                    }
                });
                let line = match target {
                    Some(jump) => self.jump(jump),
                    None if breaks => unsupported("break outside a loop"),
                    None => unsupported("continue outside a loop"),
                };
                out.push_str(&format!("{pad}{line};\n"));
            }
            "GotoStmt" => {
                let target = stmt["targetLabelDeclId"].as_str().unwrap_or("");
                match self.labels.get(target) {
                    Some(label) => {
                        let set = format!("{pad}{} = {};\n", label.var, label.segment);
                        let jump = Jump {
                            scope: label.scope,
                            continues: true,
                        };
                        out.push_str(&set);
                        let line = self.jump(jump);
                        out.push_str(&format!("{pad}{line};\n"));
                    }
                    None => out.push_str(&format!("{pad}{};\n", unsupported("goto into a block"))),
                }
            }
            // A label outside a block's own statements is no goto target.
            "LabelStmt" => self.stmt(&args[0], depth, out),
            _ => return false,
        }
        true
    }

    /// A loop whose `continue` still runs `step`, the increment of a `for`
    /// or the test of a `do`: the body is a labelled block that `continue`
    /// leaves.
    fn stepped_loop(
        &mut self,
        header: &str,
        body: &Value,
        step: &str,
        depth: usize,
        out: &mut String,
    ) {
        let pad = "    ".repeat(depth);
        let scope = self.push_scope(true);
        let block = self.push_scope(false);
        self.loops.push(CLoop {
            break_: Jump {
                scope,
                continues: false,
            },
            continue_: Jump {
                scope: block,
                continues: false,
            },
        });
        let mut lines = String::new();
        self.body(body, depth + 1, &mut lines);
        self.loops.pop();
        let block_label = self.pop_scope();
        let label = self.pop_scope();
        let lines = if block_label.is_empty() {
            lines
        } else {
            format!("{pad}    {block_label}{{\n{}{pad}    }}\n", indent(&lines))
        };
        out.push_str(&format!("{pad}{label}{header} {{\n{lines}{step}{pad}}}\n"));
    }

    /// A block with labels: its statements split at each label into
    /// segments, which a loop runs from the segment a variable names.
    fn segments(&mut self, stmts: &[Value], depth: usize, out: &mut String) {
        let pad = "    ".repeat(depth);
        let var = format!("__goto{}", self.fresh);
        let scope = self.push_scope(true);
        let mut segments: Vec<Vec<&Value>> = vec![Vec::new()];
        let mut ids = Vec::new();
        for stmt in stmts {
            let mut stmt = stmt;
            if kind(stmt) == "LabelStmt" && segments.last().is_some_and(|s| !s.is_empty()) {
                segments.push(Vec::new());
            }
            while kind(stmt) == "LabelStmt" {
                let id = stmt["declId"].as_str().unwrap_or("").to_string();
                self.labels.insert(
                    id.clone(),
                    Label {
                        scope,
                        var: var.clone(),
                        segment: segments.len() - 1,
                    },
                );
                ids.push(id);
                stmt = &inner(stmt)[0];
            }
            if let Some(segment) = segments.last_mut() {
                segment.push(stmt);
            }
        }
        let mut hoisted = String::new();
        let mut arms = String::new();
        let arm_pad = format!("{pad}        ");
        for (i, segment) in segments.iter().enumerate() {
            let last = i + 1 == segments.len();
            let pattern = if last { "_".to_string() } else { i.to_string() };
            arms.push_str(&format!("{arm_pad}{pattern} => {{\n"));
            for stmt in segment {
                if kind(stmt) == "DeclStmt" {
                    // The segments are separate Rust blocks, which would end
                    // the variable's scope early, so the variable is
                    // declared before them and its declaration assigns it.
                    for var in inner(stmt) {
                        match self.hoist(var, stmts) {
                            Some((decl, assign)) => {
                                hoisted.push_str(&format!("{pad}{decl}\n"));
                                if let Some(assign) = assign {
                                    arms.push_str(&format!("{arm_pad}    {assign};\n"));
                                }
                            }
                            None => arms.push_str(&format!(
                                "{arm_pad}    {};\n",
                                unsupported("declaration in a block with labels")
                            )),
                        }
                    }
                } else {
                    self.stmt(stmt, depth + 3, &mut arms);
                }
            }
            if !segment.last().is_some_and(|s| jumps(s)) {
                if last {
                    let line = self.jump(Jump {
                        scope,
                        continues: false,
                    });
                    arms.push_str(&format!("{arm_pad}    {line};\n"));
                } else {
                    arms.push_str(&format!("{arm_pad}    {var} = {};\n", i + 1));
                }
            }
            arms.push_str(&format!("{arm_pad}}}\n"));
        }
        for id in ids {
            self.labels.remove(&id);
        }
        let label = self.pop_scope();
        out.push_str(&hoisted);
        out.push_str(&format!(
            "{pad}let mut {var} = 0;\n{pad}{label}loop {{\n{pad}    match {var} {{\n{arms}{pad}    }}\n{pad}}}\n"
        ));
    }

    /// The declaration before a block's segments of a variable
    /// declared in it, and the assignment of its initialiser where it was
    /// declared. None if declaring it earlier would hide another variable
    /// of the same name that the block uses.
    fn hoist(&mut self, var: &Value, block: &[Value]) -> Option<(String, Option<String>)> {
        if kind(var) != "VarDecl"
            || !var["storageClass"].is_null()
            || value_type(c_type(var)).is_none()
        {
            return None;
        }
        let (name, id) = (var["name"].as_str()?, var["id"].as_str()?);
        if block.iter().any(|stmt| refers_to_other(stmt, name, id)) {
            return None;
        }
        let binding = self.binding(var);
        let zero = zero(c_type(var));
        match inner(var).first() {
            Some(init) if var["init"].is_string() => {
                let binding = binding.strip_prefix("mut ").unwrap_or(&binding);
                let assign = format!("{} = {}", ident(name), self.expr(init));
                Some((format!("let mut {binding} = {zero};"), Some(assign)))
            }
            _ => Some((format!("let {binding} = {zero};"), None)),
        }
    }
}

/// Whether `node` uses a variable named `name` other than the one with `id`.
fn refers_to_other(node: &Value, name: &str, id: &str) -> bool {
    let decl = &node["referencedDecl"];
    (kind(node) == "DeclRefExpr" && decl["name"] == name && decl["id"] != id)
        || inner(node)
            .iter()
            .any(|child| refers_to_other(child, name, id))
}
