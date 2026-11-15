//! `found`, R40: a local pointer that only ever holds what a search of the
//! library found, `strchr` and its like, and that is only tested for NULL,
//! is whether the search found something and where: `Type::Found`, an
//! `Option<usize>` in Rust. A local that is used in any other way stays a
//! pointer, and the analysis refuses the library's pointer in it.

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
            let check = Check { program, var };
            let init_ok = declared.init.as_ref().is_none_or(|init| check.value(init));
            // A pointer that only ever holds NULL is no search's.
            let searched = declared.init.iter().any(|init| is_search(program, init))
                || crate::c::any_expr(&body, &program.vars, &mut |expr| {
                    matches!(&expr.kind, ExprKind::Assign { op: None, target, value }
                        if matches!(target.kind, ExprKind::Var(named) if named == var)
                            && is_search(program, value))
                });
            if init_ok && searched && check.stmts(&body) {
                retype(&mut body, &mut program.vars, var);
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
