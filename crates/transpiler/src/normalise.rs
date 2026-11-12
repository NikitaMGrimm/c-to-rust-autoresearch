//! Rewrites of the C tree that state what the program means in a form the
//! rules can translate one statement at a time. They run between two rounds
//! of the analysis: they read the facts of the tree as written, and the
//! rules read the facts of the tree they leave.
//!
//! The one rewrite so far, R10, names the pointer that a test has found not
//! to be NULL. C tests `p` and goes on using `p`; Rust opens an `Option`
//! once and uses what was in it, as `if let Some(p) = p` and
//! `let Some(p) = p else { return; }` do.

use crate::c::{
    BinOp, Expr, ExprKind, Program, Stmt, UnOp, Var, VarId, VarKind, any_expr, each_expr_mut,
};
use crate::facts::{Facts, Slot};
use std::collections::HashSet;

pub fn narrow(program: &mut Program, facts: &Facts) {
    // A variable whose address is taken may change where no assignment
    // names it.
    let mut escaped = HashSet::new();
    for function in &program.functions {
        any_expr(&function.body, &program.vars, &mut |expr| {
            if let ExprKind::AddrOf(place) = &expr.kind
                && let Some(var) = place.root_var()
            {
                escaped.insert(var);
            }
            false
        });
    }
    for function in 0..program.functions.len() {
        let body = std::mem::take(&mut program.functions[function].body);
        let mut narrow = Narrow {
            vars: &mut program.vars,
            facts,
            escaped: &escaped,
        };
        program.functions[function].body = narrow.block(body);
    }
}

struct Narrow<'a> {
    vars: &'a mut Vec<Var>,
    facts: &'a Facts,
    escaped: &'a HashSet<VarId>,
}

impl Narrow<'_> {
    fn block(&mut self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        let mut out = Vec::new();
        let mut rest = stmts.into_iter();
        while let Some(stmt) = rest.next() {
            let Stmt::If(cond, then, other) = stmt else {
                out.push(self.inside(stmt));
                continue;
            };
            let Some((var, source, set_in_then)) = self.tested(&cond) else {
                out.push(Stmt::If(cond, self.block(then), self.block(other)));
                continue;
            };
            let (mut some, none) = if set_in_then {
                (then, other)
            } else {
                (other, then)
            };
            if some.is_empty() && leaves(&none) {
                // A guard: the pointer is set in all that follows.
                let mut after: Vec<Stmt> = rest.collect();
                if assigns(&after, self.vars, var) {
                    out.push(guard(cond, set_in_then, none));
                    out.extend(self.block(after));
                    return out;
                }
                let set = self.rename(&mut after, var);
                out.push(Stmt::LetSome {
                    var: set,
                    source,
                    otherwise: self.block(none),
                });
                out.extend(self.block(after));
                return out;
            }
            if some.is_empty() || assigns(&some, self.vars, var) {
                let (then, other) = if set_in_then {
                    (some, none)
                } else {
                    (none, some)
                };
                out.push(Stmt::If(cond, self.block(then), self.block(other)));
                continue;
            }
            let set = self.rename(&mut some, var);
            out.push(Stmt::IfSome {
                var: set,
                source,
                then: self.block(some),
                other: self.block(none),
            });
        }
        out
    }

    /// The statement with the blocks inside it rewritten.
    fn inside(&mut self, stmt: Stmt) -> Stmt {
        match stmt {
            Stmt::Block(body) => Stmt::Block(self.block(body)),
            Stmt::While(cond, body) => Stmt::While(cond, self.block(body)),
            Stmt::DoWhile(body, cond) => Stmt::DoWhile(self.block(body), cond),
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => Stmt::For {
                init,
                cond,
                step,
                body: self.block(body),
            },
            // A `case` label is no start of a block: the statements before
            // it need not have run.
            other => other,
        }
    }

    /// The variable that the condition tests against NULL, an expression
    /// that reads it, and whether the condition holds when it is set.
    fn tested(&self, cond: &Expr) -> Option<(VarId, Expr, bool)> {
        let null = |expr: &Expr| matches!(expr.uncast().kind, ExprKind::Null);
        let (pointer, set) = match &cond.kind {
            ExprKind::Unary(UnOp::Not, inner) => (&**inner, false),
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne), left, right) if null(right) => {
                (&**left, *op == BinOp::Ne)
            }
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne), left, right) if null(left) => {
                (&**right, *op == BinOp::Ne)
            }
            _ => (cond, true),
        };
        let pointer = match &pointer.kind {
            ExprKind::Cast(inner) if inner.ty.is_pointer() => inner,
            _ => pointer,
        };
        let ExprKind::Var(var) = pointer.kind else {
            return None;
        };
        let facts = self.facts.pointer(Slot::Var(var));
        let narrows = pointer.ty.is_pointer()
            && !self.vars[var].kind.is_global()
            && !self.escaped.contains(&var)
            && facts.nullable
            && facts.conflict.is_none();
        narrows.then(|| (var, pointer.clone(), set))
    }

    /// Makes a variable for `var` where it is set, and names it in the
    /// statements in place of `var`.
    fn rename(&mut self, stmts: &mut [Stmt], var: VarId) -> VarId {
        let set = self.vars.len();
        self.vars.push(Var {
            init: None,
            kind: VarKind::Local,
            ..self.vars[var].clone()
        });
        each_expr_mut(stmts, self.vars, &mut |expr| {
            if matches!(expr.kind, ExprKind::Var(named) if named == var) {
                expr.kind = ExprKind::Var(set);
            }
        });
        set
    }
}

/// The `if` of a guard as it was.
fn guard(cond: Expr, set_in_then: bool, none: Vec<Stmt>) -> Stmt {
    match set_in_then {
        true => Stmt::If(cond, Vec::new(), none),
        false => Stmt::If(cond, none, Vec::new()),
    }
}

/// Whether the statements end by leaving the block they are in.
fn leaves(stmts: &[Stmt]) -> bool {
    match stmts.last() {
        Some(Stmt::Return(_) | Stmt::Break | Stmt::Continue | Stmt::Goto(_)) => true,
        Some(Stmt::Expr(expr)) => match &expr.kind {
            ExprKind::Call(callee, _) => matches!(callee.function(), Some("exit" | "abort")),
            _ => false,
        },
        _ => false,
    }
}

/// Whether the statements assign to the variable.
fn assigns(stmts: &[Stmt], vars: &[Var], var: VarId) -> bool {
    any_expr(stmts, vars, &mut |expr| match &expr.kind {
        ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => {
            matches!(target.kind, ExprKind::Var(named) if named == var)
        }
        _ => false,
    })
}
