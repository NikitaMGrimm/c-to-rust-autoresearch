//! Rewrites of the C tree that state what the program means in a form the
//! rules can translate one statement at a time. They run between two rounds
//! of the analysis: they read the facts of the tree as written, and the
//! rules read the facts of the tree they leave.
//!
//! `unions`, R33, makes a union that holds the bits of a number the integer
//! of its size. It runs first, so that no other stage sees the union; it is
//! in a file of its own.
//!
//! `voids`, R43, gives a `void` pointer the one type that it holds. It
//! runs after `unions`, so that no later stage sees a `void` pointer; it
//! is in a file of its own.
//!
//! `places`, R62, reads a global pointer that never changes and that
//! holds the address of a global, or of a part of one at a constant place,
//! as that address: every use is the initialiser, and the pointer goes.
//! It runs before the first analysis, so that no fact sees the pointer.
//!
//! `join` gives a local its first value where it is declared: C89 declares
//! at the top of a block and assigns later, and Rust declares where the
//! value is known. It needs no facts and runs before the first analysis.
//!
//! `ends`, R21, makes the end of a number that `strtol` stores through a
//! pointer a result of the call. It needs no facts either, and is in a file
//! of its own.
//!
//! `found`, R40, makes a local that only holds what a search of the
//! library found, and is only tested, whether and where it found it. It
//! needs no facts and is in a file of its own.
//!
//! `same`, R53, gives a call that hands one pointer in several arguments
//! a copy of the function in which those parameters are one; it is in a
//! file of its own.
//!
//! `cursors`, R24, makes a pointer that walks an array an index beside the
//! array; it is in a file of its own.
//!
//! `narrow`, R10, names the pointer that a test has found not to be NULL.
//! C tests `p` and goes on using `p`; Rust opens an `Option` once and uses
//! what was in it, as `if let Some(p) = p` and
//! `let Some(p) = p else { return; }` do.

mod cursors;
mod ends;
mod errno;
mod found;
mod same;
mod unions;
mod voids;

pub use cursors::{cursors, steps_before};
pub use ends::ends;
pub use found::{SEARCHES, found, is_search};
pub use same::same;
pub use unions::unions;
pub use voids::voids;

use crate::c::{
    BinOp, Expr, ExprKind, Program, Stmt, UnOp, Var, VarId, VarKind, any_expr, each_expr_mut,
    holds_statements,
};
use crate::facts::{Facts, Slot};
use std::collections::HashSet;

/// R62: replaces every use of a global pointer that is never assigned and
/// whose address is never taken by the place its initialiser names, and
/// reads `*&x` as `x`, `(&s)->a` as `s.a` and `&x[i]` as `x[i]`'s address.
pub fn places(program: &mut Program) {
    let mut changed = HashSet::new();
    let mut mark = |expr: &Expr| {
        match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => {
                if let ExprKind::Var(var) = target.kind {
                    changed.insert(var);
                }
            }
            ExprKind::AddrOf(place) => {
                if let Some(var) = place.root_var() {
                    changed.insert(var);
                }
            }
            _ => {}
        }
        false
    };
    for function in &program.functions {
        any_expr(&function.body, &program.vars, &mut mark);
    }
    for &global in &program.globals {
        if let Some(init) = &program.vars[global].init {
            init.any(&mut mark);
        }
    }
    let fixed: Vec<(VarId, Expr)> = program
        .globals
        .iter()
        .filter_map(|&global| {
            let var = &program.vars[global];
            let init = var.init.as_ref()?;
            let named = |part: &Expr| matches!(part.kind, ExprKind::Var(v) if v == global);
            (var.ty.is_pointer()
                && var.ty.function().is_none()
                && !changed.contains(&global)
                && init.ty == var.ty
                && !init.any(&mut |part| named(part))
                && names_place(init, program))
            .then(|| (global, init.clone()))
        })
        .collect();
    if fixed.is_empty() {
        return;
    }
    let replace = |expr: &mut Expr| {
        if let ExprKind::Var(var) = expr.kind
            && let Some((_, init)) = fixed.iter().find(|(global, _)| *global == var)
        {
            *expr = init.clone();
        }
    };
    let simplify = |expr: &mut Expr| match &mut expr.kind {
        ExprKind::Deref(inner) if matches!(inner.kind, ExprKind::AddrOf(_)) => {
            let ExprKind::AddrOf(place) = std::mem::replace(&mut inner.kind, ExprKind::Zero) else {
                unreachable!()
            };
            *expr = *place;
        }
        ExprKind::Member { base, arrow, .. }
            if *arrow && matches!(base.kind, ExprKind::AddrOf(_)) =>
        {
            let ExprKind::AddrOf(place) = std::mem::replace(&mut base.kind, ExprKind::Zero) else {
                unreachable!()
            };
            **base = *place;
            *arrow = false;
        }
        _ => {}
    };
    let mut both = |expr: &mut Expr| {
        replace(expr);
        // The children are visited after the node: simplify at each level.
        simplify(expr);
    };
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        each_expr_mut(&mut body, &mut program.vars, &mut both);
        program.functions[function].body = body;
    }
    let globals = program.globals.clone();
    for global in globals {
        if let Some(init) = &mut program.vars[global].init {
            init.each_mut(&mut both);
        }
    }
    program
        .globals
        .retain(|global| !fixed.iter().any(|(var, _)| var == global));
}

/// Whether the initialiser is the address of a global, or of a field or an
/// element at a constant index of one: a place that stays where it is.
fn names_place(init: &Expr, program: &Program) -> bool {
    fn place(expr: &Expr, program: &Program) -> bool {
        match &expr.kind {
            ExprKind::Var(var) => program.vars[*var].kind.is_global(),
            ExprKind::Member {
                base, arrow: false, ..
            } => place(base, program),
            ExprKind::Index(base, index) => {
                index.constant(program).is_some()
                    && matches!(&base.kind, ExprKind::Decay(array) if place(array, program))
            }
            _ => false,
        }
    }
    match &init.kind {
        ExprKind::AddrOf(inner) | ExprKind::Decay(inner) => place(inner, program),
        _ => false,
    }
}

pub fn join(program: &mut Program) {
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        join_block(&mut body, &mut program.vars);
        program.functions[function].body = body;
    }
}

/// Whether the statement names the variable, at any depth.
fn names(stmt: &Stmt, vars: &[Var], var: VarId) -> bool {
    any_expr(
        std::slice::from_ref(stmt),
        vars,
        &mut |expr| matches!(expr.kind, ExprKind::Var(named) if named == var),
    )
}

/// The value of `var = value`, if the expression is that and the value
/// does not read the variable.
fn first_value(expr: &Expr, var: VarId) -> Option<&Expr> {
    match &expr.kind {
        ExprKind::Assign {
            op: None,
            target,
            value,
        } if matches!(target.kind, ExprKind::Var(named) if named == var)
            && !value
                .any(&mut |part| matches!(part.kind, ExprKind::Var(named) if named == var)) =>
        {
            Some(value)
        }
        _ => None,
    }
}

fn join_block(stmts: &mut Vec<Stmt>, vars: &mut [Var]) {
    // A label can be reached without the statements before it.
    let labelled = stmts
        .iter()
        .any(|stmt| matches!(stmt, Stmt::Case(_) | Stmt::Default | Stmt::Label(_)));
    let mut index = 0;
    while !labelled && index < stmts.len() {
        let Stmt::Decl(var) = stmts[index] else {
            index += 1;
            continue;
        };
        let declared = &vars[var];
        let first = (index + 1..stmts.len()).find(|&later| names(&stmts[later], vars, var));
        let (Some(first), VarKind::Local, None) = (first, declared.kind, &declared.init) else {
            index += 1;
            continue;
        };
        let used_after = stmts[first + 1..].iter().any(|stmt| names(stmt, vars, var));
        match &mut stmts[first] {
            // `int x; …; x = value;`
            Stmt::Expr(expr) if first_value(expr, var).is_some() => {
                vars[var].init = first_value(expr, var).cloned();
                stmts[first] = Stmt::Decl(var);
                stmts.remove(index);
            }
            // `int i; …; for (i = 0; …)`: the loop declares `i` if nothing
            // after it uses `i`, and otherwise the declaration comes just
            // before the loop.
            Stmt::For { init, .. } if matches!(init.as_slice(), [Stmt::Expr(expr)] if first_value(expr, var).is_some()) =>
            {
                if let [Stmt::Expr(expr)] = init.as_slice() {
                    vars[var].init = first_value(expr, var).cloned();
                }
                match used_after {
                    true => {
                        init.clear();
                        stmts.insert(first, Stmt::Decl(var));
                    }
                    false => *init = vec![Stmt::Decl(var)],
                }
                stmts.remove(index);
            }
            _ => index += 1,
        }
    }
    for stmt in stmts {
        match stmt {
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body)
            | Stmt::For { body, .. } => join_block(body, vars),
            Stmt::If(_, then, other) => {
                join_block(then, vars);
                join_block(other, vars);
            }
            _ => {}
        }
    }
}

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
        if holds_statements(&program.functions[function].body, &program.vars) {
            continue;
        }
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
                // A label among them is reached without the guard too.
                let labelled = after.iter().any(|stmt| matches!(stmt, Stmt::Label(_)));
                if labelled || assigns(&after, self.vars, var) {
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
            len: None,
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
pub(crate) fn leaves(stmts: &[Stmt]) -> bool {
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
