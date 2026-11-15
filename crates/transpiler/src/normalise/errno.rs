//! `errno`, R37: what a call of the library reports there becomes a result
//! of the call. A program stores 0, calls and reads; here the call returns
//! the error beside its value, the store is dropped, and each read that
//! the call reaches becomes a test of that result.
//!
//! A read means the error of one call only where it follows `errno = 0`
//! and that call, in the statements of one block and in the branches of an
//! `if` there, with nothing between them that may set `errno`. Any other
//! read is left as it is, and refused.

use crate::c::{BinOp, Expr, ExprKind, Program, Stmt, Type, UnOp, Var, VarId, VarKind, any_expr};

use super::ends::{PARSERS, declared};

/// The library functions that leave `errno` as it is, on which a read may
/// follow its call across them.
const LEAVE_ERRNO: [&str; 24] = [
    "printf", "fprintf", "puts", "putchar", "fputs", "fputc", "putc", "strlen", "strcmp",
    "strncmp", "memcmp", "memcpy", "memmove", "memset", "strcpy", "strncpy", "strcat", "strncat",
    "abs", "labs", "llabs", "fabs", "floor", "ceil",
];

const ERANGE: i128 = 34;

pub fn errno(program: &mut Program) {
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        let mut scope = program.functions[function].params.clone();
        scope.extend(&program.globals);
        declared(&body, &mut scope);
        let taken = scope
            .iter()
            .map(|&var| program.vars[var].name.clone())
            .collect();
        let own = program.functions.iter().map(|f| f.name.clone()).collect();
        let mut pass = Errno {
            vars: &mut program.vars,
            taken,
            own,
        };
        pass.block(&mut body, Reach::None);
        program.functions[function].body = body;
    }
}

/// What `errno` holds before a statement.
#[derive(Clone)]
enum Reach {
    /// Nothing that the rewrite knows.
    None,
    /// The 0 of `errno = 0`, with no call since.
    Cleared,
    /// The error of one call.
    Error(Error),
}

/// The call whose error a read means.
#[derive(Clone)]
enum Error {
    /// A parser of numbers, and the variable that is true where the
    /// number was beyond its type.
    Range(VarId),
}

struct Errno<'a> {
    vars: &'a mut Vec<Var>,
    /// The names in whose scope the function is.
    taken: Vec<String>,
    /// The program's own functions.
    own: Vec<String>,
}

/// `errno = 0;`
fn is_clear(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::Expr(Expr { kind: ExprKind::Assign { op: None, target, value }, .. })
        if target.is_errno() && matches!(value.uncast().kind, ExprKind::Int(0)))
}

impl Errno<'_> {
    /// Rewrites the statements of a block that starts with `reach`, and
    /// returns what reaches its end.
    fn block(&mut self, stmts: &mut Vec<Stmt>, mut reach: Reach) -> Reach {
        let mut index = 0;
        while index < stmts.len() {
            if is_clear(&stmts[index]) {
                // A read that is not one call's error is refused, so the
                // store only matters to the reads that this rewrites.
                stmts.remove(index);
                reach = Reach::Cleared;
                continue;
            }
            if matches!(reach, Reach::Cleared)
                && self.read_after(&stmts[index + 1..])
                && let Some(error) = self.call(&mut stmts[index])
            {
                reach = Reach::Error(error);
            } else {
                reach = self.stmt(&mut stmts[index], reach);
            }
            index += 1;
        }
        reach
    }

    /// Gives the call of the statement a result for its error, if it is one
    /// whose error has a rule.
    fn call(&mut self, stmt: &mut Stmt) -> Option<Error> {
        let Stmt::Parse { error, call, .. } = stmt else {
            return None;
        };
        let ExprKind::Call(callee, _) = &call.kind else {
            return None;
        };
        if error.is_some()
            || !callee
                .function()
                .is_some_and(|name| PARSERS.contains(&name))
        {
            return None;
        }
        let var = self.local("range", call.at);
        *error = Some(var);
        Some(Error::Range(var))
    }

    fn local(&mut self, name: &str, at: u32) -> VarId {
        let mut name = name.to_owned();
        while self.taken.contains(&name) {
            name.push('_');
        }
        self.vars.push(Var {
            name,
            ty: Type::Bool,
            init: None,
            len: None,
            kind: VarKind::Local,
            at,
        });
        self.vars.len() - 1
    }

    /// Whether a statement reads `errno` before the next that may set it.
    fn read_after(&self, rest: &[Stmt]) -> bool {
        for stmt in rest {
            if is_clear(stmt) {
                return false;
            }
            if any_expr(std::slice::from_ref(stmt), self.vars, &mut |expr| {
                expr.is_errno()
            }) {
                return true;
            }
            if self.sets(std::slice::from_ref(stmt)) {
                return false;
            }
        }
        false
    }

    /// Whether the statements may set `errno`: they call a function of the
    /// program or one of the library that is not known to leave it alone,
    /// or store to it.
    fn sets(&self, stmts: &[Stmt]) -> bool {
        any_expr(stmts, self.vars, &mut |expr| self.sets_expr(expr))
    }

    fn sets_expr(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Call(callee, _) => match callee.function() {
                Some("__errno_location") => false,
                Some(name) => {
                    self.own.iter().any(|own| own == name) || !LEAVE_ERRNO.contains(&name)
                }
                None => true,
            },
            ExprKind::Assign { target, .. } => target.is_errno(),
            _ => false,
        }
    }

    /// Rewrites the reads of one statement, and returns what reaches the
    /// statement after it.
    fn stmt(&mut self, stmt: &mut Stmt, reach: Reach) -> Reach {
        let sets = self.sets(std::slice::from_ref(stmt));
        let after = if sets { Reach::None } else { reach.clone() };
        // The value of a `switch` is a number, not a test.
        let switch = matches!(stmt, Stmt::Switch(..));
        let error = match &reach {
            Reach::Error(error) => Some(error.clone()),
            _ => None,
        };
        match stmt {
            Stmt::Block(body) => {
                self.block(body, reach);
            }
            Stmt::If(cond, then, other) => {
                // The branches start after the test.
                let start = match cond.any(&mut |expr| self.sets_expr(expr)) {
                    true => Reach::None,
                    false => {
                        if let Some(error) = &error {
                            self.reads(cond, error, true);
                        }
                        reach
                    }
                };
                self.block(then, start.clone());
                self.block(other, start);
            }
            // A loop or a `switch` that may set `errno` may run after itself.
            Stmt::While(cond, body) | Stmt::DoWhile(body, cond) | Stmt::Switch(cond, body) => {
                if let (Some(error), false) = (&error, sets) {
                    self.reads(cond, error, !switch);
                }
                self.block(body, if sets { Reach::None } else { reach });
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                if let (Some(error), false) = (&error, sets) {
                    for stmt in init.iter_mut() {
                        self.plain(stmt, error);
                    }
                    if let Some(cond) = cond {
                        self.reads(cond, error, true);
                    }
                    if let Some(step) = step {
                        self.reads(step, error, false);
                    }
                }
                self.block(body, if sets { Reach::None } else { reach });
            }
            stmt => {
                if let (Some(error), false) = (&error, sets) {
                    self.plain(stmt, error);
                }
            }
        }
        after
    }

    /// The reads in a statement without blocks.
    fn plain(&mut self, stmt: &mut Stmt, error: &Error) {
        match stmt {
            Stmt::Expr(expr) | Stmt::Return(Some(expr)) => self.reads(expr, error, false),
            Stmt::Decl(var) => {
                let var = *var;
                if let Some(mut init) = self.vars[var].init.take() {
                    self.reads(&mut init, error, false);
                    self.vars[var].init = Some(init);
                }
            }
            _ => {}
        }
    }

    /// Rewrites each read of `errno` that asks whether the call reported
    /// an error, or which one. `test` says that the value is a test.
    fn reads(&mut self, expr: &mut Expr, error: &Error, test: bool) {
        if let Some(read) = self.read(expr, error, test) {
            *expr = read;
            return;
        }
        if let ExprKind::Cond(cond, then, other) = &mut expr.kind {
            self.reads(cond, error, true);
            self.reads(then, error, test);
            self.reads(other, error, test);
            return;
        }
        let tests = matches!(&expr.kind, ExprKind::Unary(UnOp::Not, _))
            || matches!(&expr.kind, ExprKind::Binary(op, ..) if op.is_logical());
        for child in expr.children_mut() {
            self.reads(child, error, tests);
        }
        if let Some(folded) = fold(expr) {
            *expr = folded;
        }
    }

    /// The test that stands for a read of `errno`: `errno == ERANGE`,
    /// `errno != 0`, `!errno`, or `errno` as a test.
    fn read(&mut self, expr: &Expr, error: &Error, test: bool) -> Option<Expr> {
        let (code, equal) = match &expr.kind {
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne), left, right) => {
                let other = match (left.is_errno(), right.is_errno()) {
                    (true, false) => right,
                    (false, true) => left,
                    _ => return None,
                };
                let ExprKind::Int(code) = other.uncast().kind else {
                    return None;
                };
                (code, *op == BinOp::Eq)
            }
            ExprKind::Unary(UnOp::Not, inner) if inner.is_errno() => (0, true),
            _ if test && expr.is_errno() => (0, false),
            _ => return None,
        };
        // Whether `errno` holds `code`, or any error for 0.
        let reports = self.reports(error, (code != 0).then_some(code));
        let holds = match (reports, code == 0) {
            (Some(flag), false) => flag,
            (Some(flag), true) => Expr {
                ty: Type::Bool,
                kind: ExprKind::Unary(UnOp::Not, Box::new(flag)),
                at: expr.at,
            },
            (None, zero) => constant(zero, expr),
        };
        let value = match (equal, holds) {
            (true, holds) => holds,
            (
                false,
                Expr {
                    kind: ExprKind::Unary(UnOp::Not, flag),
                    ..
                },
            ) => *flag,
            (false, holds) => Expr {
                ty: Type::Bool,
                kind: ExprKind::Unary(UnOp::Not, Box::new(holds)),
                at: expr.at,
            },
        };
        Some(match expr.ty {
            Type::Bool => value,
            _ => Expr {
                ty: expr.ty.clone(),
                kind: ExprKind::Cast(Box::new(value)),
                at: expr.at,
            },
        })
    }

    /// The test that the call reported `code`, or any error for `None`;
    /// `None` where it never does.
    fn reports(&self, error: &Error, code: Option<i128>) -> Option<Expr> {
        match error {
            Error::Range(var) => matches!(code, None | Some(ERANGE)).then(|| Expr {
                ty: Type::Bool,
                kind: ExprKind::Var(*var),
                at: 0,
            }),
        }
    }
}

/// The truth of a test that a read became, if it is a constant.
fn truth(expr: &Expr) -> Option<bool> {
    match &expr.uncast().kind {
        ExprKind::Int(value) if expr.uncast().ty == Type::Bool => Some(*value != 0),
        _ => None,
    }
}

/// `&&` or `||` with a side that a read made a constant, without it: the
/// lints refuse `false || false`. The other side is kept where C
/// evaluates it and it has an effect.
fn fold(expr: &Expr) -> Option<Expr> {
    let ExprKind::Binary(op @ (BinOp::And | BinOp::Or), left, right) = &expr.kind else {
        return None;
    };
    let decides = *op == BinOp::Or;
    let (value, other) = match (truth(left), truth(right)) {
        (Some(value), _) => (value, right),
        (None, Some(value)) if left.is_pure() => (value, left),
        _ => return None,
    };
    let test = match value == decides {
        true => constant(decides, expr),
        false if other.uncast().ty == Type::Bool => (**other).clone(),
        false => Expr {
            ty: expr.ty.clone(),
            kind: ExprKind::Binary(
                BinOp::Ne,
                other.clone(),
                Box::new(Expr {
                    ty: other.ty.clone(),
                    kind: ExprKind::Int(0),
                    at: other.at,
                }),
            ),
            at: expr.at,
        },
    };
    Some(match test.ty == expr.ty {
        true => test,
        false => Expr {
            ty: expr.ty.clone(),
            kind: ExprKind::Cast(Box::new(test)),
            at: expr.at,
        },
    })
}

fn constant(value: bool, at: &Expr) -> Expr {
    Expr {
        ty: Type::Bool,
        kind: ExprKind::Int(i128::from(value)),
        at: at.at,
    }
}
