//! `found`, R40: a local pointer that only ever holds what a search of the
//! library found, `strchr` and its like, and that is only tested for NULL,
//! is whether the search found something and where: `Type::Found`, an
//! `Option<usize>` in Rust. A local that is used in any other way stays a
//! pointer, and the analysis refuses the library's pointer in it.

use super::cursors::{address, as_index, element};
use crate::c::{BinOp, Expr, ExprKind, Program, Stmt, Type, UnOp, Var, VarId, VarKind};

/// The library functions that return a place in their first argument, or
/// NULL.
pub const SEARCHES: [&str; 5] = ["strchr", "strrchr", "memchr", "strstr", "strpbrk"];

/// Whether the expression is a call of one of the library's searches.
pub fn is_search(program: &Program, expr: &Expr) -> bool {
    matches!(&expr.uncast().kind, ExprKind::Call(callee, _)
    if callee.function().is_some_and(|name| {
        SEARCHES.contains(&name) && program.function(name).is_none()
    }))
}

pub fn found(program: &mut Program) {
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        let mut locals = Vec::new();
        declared(&body, &mut locals);
        for var in locals {
            let declared = &program.vars[var];
            if declared.kind != VarKind::Local || !declared.ty.is_pointer() {
                continue;
            }
            let check = Check {
                program,
                var,
                base: None,
            };
            let init_ok = declared.init.as_ref().is_none_or(|init| check.value(init));
            // A pointer that only ever holds NULL is no search's.
            let searched = declared.init.iter().any(|init| is_search(program, init))
                || crate::c::any_expr(&body, &program.vars, &mut |expr| {
                    matches!(&expr.kind, ExprKind::Assign { op: None, target, value }
                        if matches!(target.kind, ExprKind::Var(named) if named == var)
                            && is_search(program, value))
                });
            if !(init_ok && searched) {
                continue;
            }
            if check.stmts(&body) {
                retype(&mut body, &mut program.vars, var);
                continue;
            }
            // R47: used as a pointer only where a test found it.
            let Some(base) = base_of(program, var, &body) else {
                continue;
            };
            let check = Check {
                base: Some(base),
                ..check
            };
            if check.stmts(&body) {
                retype(&mut body, &mut program.vars, var);
                let at = program.vars[var].at;
                let array = Expr {
                    ty: program.vars[base].ty.clone(),
                    kind: ExprKind::Var(base),
                    at,
                };
                // An array is read through the pointer to its first
                // element, as Clang writes it.
                let base = match array.ty.clone() {
                    Type::Array(element, _) => Expr {
                        ty: Type::Pointer(element),
                        kind: ExprKind::Decay(Box::new(array)),
                        at,
                    },
                    _ => array,
                };
                narrow(&mut body, program, var, &base);
            }
        }
        program.functions[function].body = body;
    }
}

fn declared(stmts: &[Stmt], found: &mut Vec<VarId>) {
    for stmt in stmts {
        match stmt {
            Stmt::Decl(var) => found.push(*var),
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body) => declared(body, found),
            Stmt::If(_, then, other) | Stmt::IfSome { then, other, .. } => {
                declared(then, found);
                declared(other, found);
            }
            Stmt::For { init, body, .. } => {
                declared(init, found);
                declared(body, found);
            }
            Stmt::LetSome { otherwise, .. } => declared(otherwise, found),
            _ => {}
        }
    }
}

struct Check<'a> {
    program: &'a Program,
    var: VarId,
    /// The array that every search of the variable searches, where the
    /// variable may be used as a place in it under a test (R47).
    base: Option<VarId>,
}

impl Check<'_> {
    fn is_var(&self, expr: &Expr) -> bool {
        matches!(expr.uncast().kind, ExprKind::Var(var) if var == self.var)
    }

    fn mentions(&self, expr: &Expr) -> bool {
        expr.any(&mut |part| matches!(part.kind, ExprKind::Var(var) if var == self.var))
    }

    /// Whether a value stored in the variable is a search that does not
    /// read the variable, or NULL.
    fn value(&self, value: &Expr) -> bool {
        matches!(value.uncast().kind, ExprKind::Null)
            || (is_search(self.program, value) && !self.mentions(value))
    }

    fn stmts(&self, stmts: &[Stmt]) -> bool {
        stmts.iter().all(|stmt| self.stmt(stmt))
    }

    fn stmt(&self, stmt: &Stmt) -> bool {
        let vars: &[Var] = &self.program.vars;
        match stmt {
            Stmt::Expr(expr) => match &expr.kind {
                ExprKind::Assign {
                    op: None,
                    target,
                    value,
                } if matches!(target.kind, ExprKind::Var(var) if var == self.var) => {
                    self.value(value)
                }
                _ => self.expr(expr, false),
            },
            // The variable's own initialiser is checked by `found`.
            Stmt::Decl(var) if *var == self.var => true,
            Stmt::Decl(var) => vars[*var]
                .init
                .iter()
                .chain(&vars[*var].len)
                .all(|part| self.expr(part, false)),
            Stmt::Block(body) => self.stmts(body),
            Stmt::If(cond, then, other) if self.base.is_some() && self.tested(cond).is_some() => {
                let (found, missing) = match self.tested(cond) {
                    Some(true) => (then, other),
                    _ => (other, then),
                };
                self.placed(found) && self.stmts(missing)
            }
            Stmt::If(cond, then, other) => {
                self.expr(cond, true) && self.stmts(then) && self.stmts(other)
            }
            Stmt::While(cond, body) | Stmt::DoWhile(body, cond) => {
                self.expr(cond, true) && self.stmts(body)
            }
            Stmt::Switch(value, body) => self.expr(value, false) && self.stmts(body),
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.stmts(init)
                    && cond.iter().all(|cond| self.expr(cond, true))
                    && step.iter().all(|step| self.expr(step, false))
                    && self.stmts(body)
            }
            Stmt::Return(Some(value)) => self.expr(value, false),
            // The test of another place that was found (R47).
            Stmt::IfSome {
                source,
                then,
                other,
                ..
            } if source.ty == Type::Found => self.stmts(then) && self.stmts(other),
            Stmt::IfSome { .. } | Stmt::LetSome { .. } | Stmt::Parse { .. } => !crate::c::any_expr(
                std::slice::from_ref(stmt),
                vars,
                &mut |part| matches!(part.kind, ExprKind::Var(var) if var == self.var),
            ),
            Stmt::Return(None)
            | Stmt::Case(_)
            | Stmt::Default
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Goto(_)
            | Stmt::Label(_) => true,
        }
    }

    /// Whether the condition is only a test of the variable: `p` and
    /// `p != NULL` are true where it was found, `!p` and `p == NULL` where
    /// it was not.
    fn tested(&self, cond: &Expr) -> Option<bool> {
        let null = |expr: &Expr| matches!(expr.uncast().kind, ExprKind::Null);
        match &cond.kind {
            _ if self.is_var(cond) => Some(true),
            ExprKind::Unary(UnOp::Not, inner) => self.tested(inner).map(|found| !found),
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne), left, right)
                if (self.is_var(left) && null(right)) || (self.is_var(right) && null(left)) =>
            {
                Some(*op == BinOp::Ne)
            }
            ExprKind::Cast(inner) if cond.ty == Type::Bool => self.tested(inner),
            _ => None,
        }
    }

    /// Whether the statements, which run where the variable was found, use
    /// it only as a place: nothing stores to it or takes its address.
    fn placed(&self, stmts: &[Stmt]) -> bool {
        !crate::c::any_expr(stmts, &self.program.vars, &mut |expr| match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => {
                self.is_var(target)
            }
            ExprKind::AddrOf(inner) => self.is_var(inner),
            _ => false,
        })
    }

    /// Whether the expression reads the variable only as a test: `test`
    /// says that its value is one.
    fn expr(&self, expr: &Expr, test: bool) -> bool {
        match &expr.kind {
            ExprKind::Var(var) if *var == self.var => test,
            ExprKind::Binary(BinOp::Eq | BinOp::Ne, left, right)
                if (self.is_var(left) && matches!(right.uncast().kind, ExprKind::Null))
                    || (self.is_var(right) && matches!(left.uncast().kind, ExprKind::Null)) =>
            {
                true
            }
            ExprKind::Unary(UnOp::Not, inner) => self.expr(inner, true),
            ExprKind::Binary(BinOp::And | BinOp::Or, left, right) => {
                self.expr(left, true) && self.expr(right, true)
            }
            ExprKind::Cond(cond, then, other) => {
                self.expr(cond, true) && self.expr(then, test) && self.expr(other, test)
            }
            ExprKind::Cast(inner) if expr.ty == Type::Bool => self.expr(inner, true),
            ExprKind::Cast(inner) => self.expr(inner, test),
            _ => expr
                .children()
                .into_iter()
                .all(|child| self.expr(child, false)),
        }
    }
}

/// Gives the variable and every expression that reads it `Type::Found`.
fn retype(body: &mut [Stmt], vars: &mut [Var], var: VarId) {
    vars[var].ty = Type::Found;
    crate::c::each_expr_mut(body, vars, &mut |expr| {
        if matches!(expr.kind, ExprKind::Var(named) if named == var) {
            expr.ty = Type::Found;
        }
    });
}

/// The array that every search stored in the variable searches: a local
/// array, or a pointer that nothing changes after its declaration, so that
/// it means the same memory wherever the place is used.
fn base_of(program: &Program, var: VarId, body: &[Stmt]) -> Option<VarId> {
    let searched = |value: &Expr| match &value.uncast().kind {
        ExprKind::Call(_, args) if is_search(program, value) => {
            let array = args.first()?.uncast();
            match &array.kind {
                ExprKind::Decay(array) => match array.kind {
                    ExprKind::Var(base) => Some(base),
                    _ => None,
                },
                ExprKind::Var(base) => Some(*base),
                _ => None,
            }
        }
        _ => None,
    };
    let mut bases: Vec<Option<VarId>> = Vec::new();
    bases.extend(
        program.vars[var]
            .init
            .iter()
            .filter(|init| is_search(program, init))
            .map(&searched),
    );
    crate::c::any_expr(body, &program.vars, &mut |expr| {
        if let ExprKind::Assign {
            op: None,
            target,
            value,
        } = &expr.kind
            && matches!(target.kind, ExprKind::Var(named) if named == var)
            && is_search(program, value)
        {
            bases.push(searched(value));
        }
        false
    });
    let base = (*bases.first()?)?;
    let is_base = |expr: &Expr| matches!(expr.kind, ExprKind::Var(named) if named == base);
    let changed = crate::c::any_expr(body, &program.vars, &mut |expr| match &expr.kind {
        ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => is_base(target),
        ExprKind::AddrOf(inner) => is_base(inner),
        _ => false,
    });
    let fixed = match program.vars[base].ty {
        Type::Array(..) => true,
        Type::Pointer(_) => !changed && !program.vars[base].kind.is_global(),
        _ => false,
    };
    (base != var && fixed && bases.iter().all(|other| *other == Some(base))).then_some(base)
}

/// R47: writes each `if` that tests the variable as a `Stmt::IfSome`, and
/// the variable in the branch where it was found as an index into `base`.
fn narrow(stmts: &mut [Stmt], program: &mut Program, var: VarId, base: &Expr) {
    for stmt in stmts {
        let check = Check {
            program,
            var,
            base: None,
        };
        match stmt {
            Stmt::If(cond, then, other) if check.tested(cond).is_some() => {
                let found = check.tested(cond) == Some(true);
                let (mut then, mut other) = (std::mem::take(then), std::mem::take(other));
                if !found {
                    std::mem::swap(&mut then, &mut other);
                }
                place(&mut then, program, var, base);
                narrow(&mut other, program, var, base);
                let source = Expr {
                    ty: Type::Found,
                    kind: ExprKind::Var(var),
                    at: cond.at,
                };
                *stmt = Stmt::IfSome {
                    var,
                    source,
                    then,
                    other,
                };
            }
            Stmt::If(_, then, other) | Stmt::IfSome { then, other, .. } => {
                narrow(then, program, var, base);
                narrow(other, program, var, base);
            }
            Stmt::LetSome { otherwise, .. } => narrow(otherwise, program, var, base),
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body)
            | Stmt::For { body, .. } => narrow(body, program, var, base),
            _ => {}
        }
    }
}

/// Writes every use of the variable in the statements as a place in
/// `base`: `*p` as `base[p]`, `p - base` as the index, and the pointer
/// itself or `p + n` as `&base[p + n]`.
fn place(stmts: &mut [Stmt], program: &mut Program, var: VarId, base: &Expr) {
    let records = &*program;
    let is_var = |expr: &Expr| {
        matches!(expr.kind, ExprKind::Var(named) if named == var) && expr.ty == Type::Found
    };
    let is_base = |expr: &Expr| {
        let expr = match &expr.uncast().kind {
            ExprKind::Decay(array) => array,
            _ => expr.uncast(),
        };
        let base = match &base.kind {
            ExprKind::Decay(array) => array,
            _ => base,
        };
        matches!((&expr.kind, &base.kind), (ExprKind::Var(named), ExprKind::Var(base)) if named == base)
    };
    let index = |at: u32| Expr {
        ty: Type::Index,
        kind: ExprKind::Var(var),
        at,
    };
    let offset = |at: u32, op: BinOp, by: &Expr| Expr {
        ty: Type::Index,
        kind: ExprKind::Binary(op, Box::new(index(at)), Box::new(as_index(by, records))),
        at,
    };
    let mut change = |expr: &mut Expr| {
        let at = expr.at;
        let changed = match &expr.kind {
            ExprKind::Deref(inner) if is_var(inner) => {
                Some(element(base.clone(), index(at), false, at).kind)
            }
            ExprKind::Index(array, by) if is_var(array) => {
                Some(element(base.clone(), offset(at, BinOp::Add, by), false, at).kind)
            }
            ExprKind::Binary(BinOp::Sub, left, right) if is_var(left) && is_base(right) => {
                Some(ExprKind::Cast(Box::new(index(at))))
            }
            ExprKind::Binary(op @ (BinOp::Add | BinOp::Sub), left, right)
                if is_var(left) && right.ty.is_integer() =>
            {
                Some(address(base.clone(), offset(at, *op, right), false, at))
            }
            _ if is_var(expr) => Some(address(base.clone(), index(at), false, at)),
            _ => None,
        };
        if let Some(changed) = changed {
            if is_var(expr) {
                expr.ty = Type::Pointer(Box::new(base.ty.element().cloned().unwrap_or(Type::Void)));
            }
            expr.kind = changed;
        }
    };
    // The initialisers of the locals that the statements declare change
    // too, in a copy while `change` reads the program.
    let mut vars = program.vars.clone();
    crate::c::each_expr_mut(stmts, &mut vars, &mut change);
    program.vars = vars;
}
