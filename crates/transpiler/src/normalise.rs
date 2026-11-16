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
//! `protos`, R63, gives a pointer to a function without a prototype the
//! parameters of the functions it holds. It runs after `voids`; it is in a
//! file of its own.
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
//! `alias`, R61, makes a local pointer that only ever names one place,
//! and that Rust would reject as a borrow, that place; it is in a file of
//! its own.
//!
//! `narrow`, R10, names the pointer that a test has found not to be NULL.
//! C tests `p` and goes on using `p`; Rust opens an `Option` once and uses
//! what was in it, as `if let Some(p) = p` and
//! `let Some(p) = p else { return; }` do.

mod alias;
mod cursors;
mod ends;
mod errno;
mod found;
mod protos;
mod same;
mod unions;
mod voids;

pub use alias::alias;
pub use cursors::{cursors, steps_before};
pub use ends::ends;
pub use found::{SEARCHES, found, is_search};
pub use protos::protos;
pub use same::same;
pub use unions::unions;
pub use voids::voids;

use crate::c::{
    BinOp, Expr, ExprKind, Program, Stmt, Type, UnOp, Var, VarId, VarKind, any_expr, each_expr_mut,
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
            .then(|| (global, folded(init, program)))
        })
        .collect();
    if !fixed.is_empty() {
        rewrite(program, |expr| {
            match expr.kind {
                ExprKind::Var(var) => fixed.iter().find(|(global, _)| *global == var),
                _ => None,
            }
            .map(|(_, init)| init.clone())
        });
        program
            .globals
            .retain(|global| !fixed.iter().any(|(var, _)| var == global));
    }
    for record in 0..program.records.len() {
        for field in (0..program.records[record].fields.len()).rev() {
            fixed_field(program, record, field);
        }
    }
}

/// R64: replaces every read `v.p` of a pointer field that no store changes
/// by the place that `v`'s initialiser gives it, and takes the field out of
/// the record. Every record of the type must be a global with an
/// initialiser, so that which one a read names, and its place, is known.
fn fixed_field(program: &mut Program, record: usize, field: usize) {
    let declared = &program.records[record];
    let ty = &declared.fields[field].ty;
    if !declared.own || declared.union || !ty.is_pointer() || ty.function().is_some() {
        return;
    }
    let ours = |ty: &Type| matches!(ty, Type::Record(r) if *r == record);
    let pointer = |ty: &Type| matches!(ty, Type::Pointer(inner) if ours(inner));
    // The records of the type and their places for the field.
    let mut places = Vec::new();
    for (id, var) in program.vars.iter().enumerate() {
        if !ours(&var.ty) {
            if holds(&var.ty, record, program) {
                return;
            }
            continue;
        }
        let Some(Expr {
            kind: ExprKind::InitList(items),
            ..
        }) = &var.init
        else {
            return;
        };
        let place = match items.get(field) {
            Some(place) if var.kind.is_global() && place.ty == *ty => place,
            _ => return,
        };
        let any_record =
            |expr: &Expr| matches!(expr.kind, ExprKind::Var(v) if ours(&program.vars[v].ty));
        if !names_place(place, program) || place.any(&mut |part| any_record(part)) {
            return;
        }
        places.push((id, folded(place, program)));
    }
    let returns = program
        .functions
        .iter()
        .any(|f| holds(&f.ret, record, program));
    let nested = program.records.iter().enumerate().any(|(other, r)| {
        other != record && r.fields.iter().any(|f| holds(&f.ty, record, program))
    });
    if places.is_empty() || returns || nested {
        return;
    }
    let this_field = |expr: &Expr| matches!(&expr.kind, ExprKind::Member { record: r, field: f, .. } if *r == record && *f == field);
    // The global that a read of the field names: `v.p`, or `(&v)->p` where
    // R62 replaced a pointer to `v`.
    let named = |expr: &Expr| match &expr.kind {
        ExprKind::Member { base, arrow, .. } if this_field(expr) => match (&base.kind, arrow) {
            (ExprKind::Var(var), false) => Some(*var),
            (ExprKind::AddrOf(inner), true) => match inner.kind {
                ExprKind::Var(var) => Some(var),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    };
    let initialiser = |expr: &Expr| {
        places.iter().any(|(var, _)| {
            program.vars[*var]
                .init
                .as_ref()
                .is_some_and(|init| std::ptr::eq(init, expr))
        })
    };
    let mut refused = |expr: &Expr| match &expr.kind {
        ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => {
            this_field(target) || ours(&target.ty)
        }
        ExprKind::AddrOf(place) => this_field(place),
        ExprKind::Member { .. } if this_field(expr) => {
            named(expr).is_none_or(|var| !places.iter().any(|(v, _)| *v == var))
        }
        // `memcpy` and `malloc` reach a record through `void *`.
        ExprKind::Cast(inner) => pointer(&expr.ty) || pointer(&inner.ty),
        ExprKind::SizeOf(ty) => holds(ty, record, program),
        ExprKind::Var(_) => false,
        ExprKind::InitList(_) if initialiser(expr) => false,
        _ => ours(&expr.ty),
    };
    let in_bodies = program
        .functions
        .iter()
        .any(|function| any_expr(&function.body, &program.vars, &mut refused));
    let in_globals = program.globals.iter().any(|&global| {
        let init = program.vars[global].init.as_ref();
        init.is_some_and(|init| init.any(&mut refused))
    });
    if in_bodies || in_globals {
        return;
    }
    rewrite(program, |expr| {
        let var = named(expr)?;
        places
            .iter()
            .find(|(v, _)| *v == var)
            .map(|(_, place)| place.clone())
    });
    program.records[record].fields.remove(field);
    let mut shift = |expr: &mut Expr| {
        if let ExprKind::Member {
            record: r,
            field: f,
            ..
        } = &mut expr.kind
            && *r == record
            && *f > field
        {
            *f -= 1;
        }
    };
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        each_expr_mut(&mut body, &mut program.vars, &mut shift);
        program.functions[function].body = body;
    }
    for (var, _) in places {
        let init = program.vars[var].init.as_mut();
        if let Some(Expr {
            kind: ExprKind::InitList(items),
            ..
        }) = init
        {
            items.remove(field);
        }
    }
    // A `static` local's initialiser is in its function's body as well.
    let globals = program.globals.clone();
    for global in globals {
        if let (VarKind::Global, Some(init)) =
            (program.vars[global].kind, &mut program.vars[global].init)
        {
            init.each_mut(&mut shift);
        }
    }
}

/// Whether a value of the type holds a record of `record`'s type, as
/// itself, an element or a field, or a function's type takes or gives one.
fn holds(ty: &Type, record: usize, program: &Program) -> bool {
    match ty {
        Type::Record(r) => {
            *r == record
                || program.records[*r]
                    .fields
                    .iter()
                    .any(|field| holds(&field.ty, record, program))
        }
        Type::Array(inner, _) => holds(inner, record, program),
        Type::Function { ret, params, .. } => {
            holds(ret, record, program) || params.iter().any(|p| holds(p, record, program))
        }
        Type::Pointer(inner) => {
            matches!(**inner, Type::Function { .. }) && holds(inner, record, program)
        }
        _ => false,
    }
}

/// Replaces each expression for which `place` gives a value by that
/// value, and reads `*&x` as `x` and `(&s)->a` as `s.a`.
fn rewrite(program: &mut Program, place: impl Fn(&Expr) -> Option<Expr>) {
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
        // `(&a[2])[n]` is `a[2 + n]`: the place is in the middle of its
        // array, and `n` may be negative.
        ExprKind::Index(base, by) => {
            if let ExprKind::AddrOf(place) = &mut base.kind
                && let ExprKind::Index(_, at) = &mut place.kind
                && let ExprKind::Int(start) = at.kind
            {
                let sum = match by.kind {
                    ExprKind::Int(more) => Some(ExprKind::Int(start + more)),
                    _ if matches!(
                        by.ty,
                        Type::Int {
                            bits: 32 | 64,
                            signed: true
                        }
                    ) =>
                    {
                        at.ty = by.ty.clone();
                        Some(ExprKind::Binary(
                            BinOp::Add,
                            at.clone(),
                            Box::new(std::mem::replace(&mut **by, (**at).clone())),
                        ))
                    }
                    _ => None,
                };
                if let Some(sum) = sum {
                    at.kind = sum;
                    let ExprKind::Index(array, at) =
                        std::mem::replace(&mut place.kind, ExprKind::Zero)
                    else {
                        unreachable!()
                    };
                    *base = array;
                    *by = at;
                }
            }
        }
        _ => {}
    };
    let mut both = |expr: &mut Expr| {
        if let Some(value) = place(expr) {
            *expr = value;
        }
        // The children are visited after the node: an index needs its base
        // now, to be an index of the array that the place is in.
        if let ExprKind::Index(base, _) = &mut expr.kind
            && let Some(value) = place(base)
        {
            **base = value;
        }
        // Simplify at each level.
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
}

/// The place with each index as its number, `&a[1 + 0]` as `&a[1]`, so
/// that an index through it adds to a number.
fn folded(place: &Expr, program: &Program) -> Expr {
    let mut place = place.clone();
    place.each_mut(&mut |expr| {
        if let ExprKind::Index(_, at) = &mut expr.kind
            && let Some(value) = at.constant(program)
        {
            at.kind = ExprKind::Int(value);
        }
    });
    place
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
