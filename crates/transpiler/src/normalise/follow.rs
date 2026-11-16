//! R66: the name follows the memory. After `q = p;` hands the memory of
//! the owner `p` to the local owner `q`, C reads the same pointer by both
//! names until one of them is stored to; Rust has moved it, and only `q`
//! names it. So each read of `p` up to the next store to either is a read
//! of `q`.

use super::alias::hides;
use crate::c::{Expr, ExprKind, Program, Stmt, VarId, VarKind, any_expr, each_expr_mut};
use crate::facts::{Facts, Slot};

/// Rewrites the functions in which `Facts::moved` finds a read of an owner
/// whose memory went to another, and says whether it rewrote any.
pub fn follow(program: &mut Program, facts: &Facts) -> bool {
    let mut changed = false;
    for function in 0..program.functions.len() {
        if facts.moved(function).is_none() {
            continue;
        }
        let mut body = std::mem::take(&mut program.functions[function].body);
        // A jump may reach a statement from where the two names differ.
        if !jumps(&body) {
            changed |= block(&mut body, program, facts);
        }
        program.functions[function].body = body;
    }
    changed
}

fn block(stmts: &mut [Stmt], program: &mut Program, facts: &Facts) -> bool {
    let mut changed = false;
    for at in 0..stmts.len() {
        let (before, after) = stmts.split_at_mut(at + 1);
        if let Some((from, to)) = handed(&before[at], program, facts) {
            changed |= rename(after, program, from, to);
        }
        changed |= match &mut before[at] {
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body) => block(body, program, facts),
            Stmt::For { init, body, .. } => {
                block(init, program, facts) | block(body, program, facts)
            }
            Stmt::If(_, then, other) => block(then, program, facts) | block(other, program, facts),
            _ => false,
        };
    }
    changed
}

/// `q = p;` or `T *q = p;` for two owners, of which `q` is a local: the
/// names `(p, q)`.
fn handed(stmt: &Stmt, program: &Program, facts: &Facts) -> Option<(VarId, VarId)> {
    let (to, value) = match stmt {
        Stmt::Decl(var) => (*var, program.vars[*var].init.as_ref()?),
        Stmt::Expr(Expr {
            kind:
                ExprKind::Assign {
                    op: None,
                    target,
                    value,
                },
            ..
        }) => match target.kind {
            ExprKind::Var(var) => (var, &**value),
            _ => return None,
        },
        _ => return None,
    };
    let ExprKind::Var(from) = value.uncast().kind else {
        return None;
    };
    let vars = &program.vars;
    let owner = |var: VarId| {
        vars[var].ty.is_pointer()
            && vars[var].len.is_none()
            && !facts.addressed(var)
            && facts.pointer(Slot::Var(var)).owned
    };
    (from != to
        && vars[to].kind == VarKind::Local
        && vars[from].ty == vars[to].ty
        && owner(from)
        && owner(to))
    .then_some((from, to))
}

/// Names `to` where the statements read `from`, up to the first statement
/// that stores to either, of which a plain store to `from` still has its
/// value renamed. Says whether it renamed any.
fn rename(stmts: &mut [Stmt], program: &mut Program, from: VarId, to: VarId) -> bool {
    // The Rust names follow the C names, so a variable that hides `to`
    // where `from` is read would be named in its place.
    if hides(stmts, &program.vars, to) {
        return false;
    }
    let global = program.vars[from].kind.is_global();
    let is = |expr: &Expr, var: VarId| matches!(expr.kind, ExprKind::Var(named) if named == var);
    let mut changed = false;
    let mut swap = |expr: &mut Expr| {
        if is(expr, from) {
            expr.kind = ExprKind::Var(to);
            changed = true;
        }
    };
    for stmt in stmts {
        // A `case` is reached from the `switch`, past the store.
        if matches!(stmt, Stmt::Case(_) | Stmt::Default) {
            break;
        }
        let one = std::slice::from_mut(stmt);
        if !stops(one, program, from, to, global) {
            each_expr_mut(one, &mut program.vars, &mut swap);
            continue;
        }
        if let Stmt::Expr(Expr {
            kind:
                ExprKind::Assign {
                    op: None,
                    target,
                    value,
                },
            ..
        }) = stmt
            && is(target, from)
            && !value.any(&mut |part| ends(part, program, from, to, global))
        {
            value.each_mut(&mut swap);
        }
        break;
    }
    changed
}

/// Whether the statements may store to one of the two names.
fn stops(stmts: &[Stmt], program: &Program, from: VarId, to: VarId, global: bool) -> bool {
    stmts
        .iter()
        .any(|stmt| matches!(stmt, Stmt::Decl(var) if *var == from || *var == to))
        || any_expr(stmts, &program.vars, &mut |expr| {
            ends(expr, program, from, to, global)
        })
}

/// Whether the expression stores to one of the two names, or calls a
/// function of the program that may store to a global.
fn ends(expr: &Expr, program: &Program, from: VarId, to: VarId, global: bool) -> bool {
    let named = |expr: &Expr| matches!(expr.kind, ExprKind::Var(var) if var == from || var == to);
    match &expr.kind {
        ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => named(target),
        ExprKind::AddrOf(place) => named(place),
        ExprKind::Init { var, .. } => *var == from || *var == to,
        ExprKind::Call(callee, _) => {
            global
                && callee
                    .function()
                    .is_none_or(|name| program.function(name).is_some())
        }
        _ => false,
    }
}

/// Whether the statements hold a label or a `goto`.
fn jumps(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|stmt| match stmt {
        Stmt::Label(_) | Stmt::Goto(_) => true,
        Stmt::Block(body)
        | Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::Switch(_, body) => jumps(body),
        Stmt::For { init, body, .. } => jumps(init) || jumps(body),
        Stmt::If(_, then, other) => jumps(then) || jumps(other),
        Stmt::IfSome { then, other, .. } => jumps(then) || jumps(other),
        Stmt::LetSome { otherwise, .. } => jumps(otherwise),
        _ => false,
    })
}
