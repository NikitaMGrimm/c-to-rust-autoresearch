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
const LEAVE_ERRNO: [&str; 27] = [
    "__builtin_isnan",
    "__builtin_isinf",
    "__builtin_isfinite",
    "printf",
    "fprintf",
    "puts",
    "putchar",
    "fputs",
    "fputc",
    "putc",
    "strlen",
    "strcmp",
    "strncmp",
    "memcmp",
    "memcpy",
    "memmove",
    "memset",
    "strcpy",
    "strncpy",
    "strcat",
    "strncat",
    "abs",
    "labs",
    "llabs",
    "fabs",
    "floor",
    "ceil",
];

const ERANGE: i128 = 34;
const EDOM: i128 = 33;

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
        let mut addressed = Vec::new();
        any_expr(&body, &program.vars, &mut |expr| {
            if let ExprKind::AddrOf(place) = &expr.kind
                && let Some(var) = place.root_var()
            {
                addressed.push(var);
            }
            false
        });
        let mut pass = Errno {
            vars: &mut program.vars,
            taken,
            own,
            addressed,
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
    /// A function of `math.h`, R38: the variable that holds its result
    /// and its arguments, variables or constants, from which the error
    /// follows.
    Math {
        function: &'static str,
        result: VarId,
        args: Vec<Expr>,
    },
}

impl Error {
    /// The variables whose store ends what the error can be told from.
    fn vars(&self) -> Vec<VarId> {
        match self {
            Error::Range(_) => Vec::new(),
            Error::Math { result, args, .. } => std::iter::once(*result)
                .chain(args.iter().filter_map(|arg| arg.uncast().root_var()))
                .collect(),
        }
    }
}

struct Errno<'a> {
    vars: &'a mut Vec<Var>,
    /// The names in whose scope the function is.
    taken: Vec<String>,
    /// The program's own functions.
    own: Vec<String>,
    /// The variables whose address the function takes.
    addressed: Vec<VarId>,
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
            if matches!(stmts[index], Stmt::Case(_) | Stmt::Default) {
                // A `switch` may start here, after no call at all.
                reach = Reach::None;
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
        if let Some(error) = self.math(stmt) {
            return Some(error);
        }
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

    /// `r = pow(x, y);` or `double r = sqrt(x);`, for a `double` `r` and
    /// arguments that are `double` variables or constants, none of them
    /// with its address taken: each read becomes a test of their values.
    fn math(&self, stmt: &Stmt) -> Option<Error> {
        let (result, value) = match stmt {
            Stmt::Decl(var) => (*var, self.vars[*var].init.as_ref()?),
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
        let ExprKind::Call(callee, args) = &value.kind else {
            return None;
        };
        let function = match callee.function()? {
            "pow" => "pow",
            "sqrt" => "sqrt",
            _ => return None,
        };
        let double = Type::Float { bits: 64 };
        let plain = |arg: &Expr| match &arg.kind {
            ExprKind::Var(var) => {
                arg.ty == double && *var != result && !self.addressed.contains(var)
            }
            _ => constant_of(arg).is_some(),
        };
        let fits = self.vars[result].ty == double
            && value.ty == double
            && !self.own.iter().any(|own| own == function)
            && !self.addressed.contains(&result)
            && args.iter().all(plain);
        fits.then(|| Error::Math {
            function,
            result,
            args: args.clone(),
        })
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
        let vars = match &reach {
            Reach::Error(error) => error.vars(),
            _ => Vec::new(),
        };
        let ends = |pass: &Self, expr: &Expr| {
            pass.sets_expr(expr)
                || matches!(&expr.kind, ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. }
                    if target.root_var().is_some_and(|var| vars.contains(&var)))
        };
        let sets = any_expr(std::slice::from_ref(stmt), self.vars, &mut |expr| {
            ends(self, expr)
        });
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
                let start = match cond.any(&mut |expr| ends(self, expr)) {
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
            Error::Math {
                function,
                result,
                args,
            } => {
                let result = Expr {
                    ty: Type::Float { bits: 64 },
                    kind: ExprKind::Var(*result),
                    at: 0,
                };
                let x = &args[0];
                // What glibc reports.
                let (domain, range) = match *function {
                    "pow" => {
                        let y = &args[1];
                        let finite = and(test("isfinite", x), test("isfinite", y));
                        (
                            and(
                                test("isnan", &result),
                                and(not(test("isnan", x)), not(test("isnan", y))),
                            ),
                            or(
                                and(test("isinf", &result), finite.clone()),
                                and(
                                    and(compare(BinOp::Eq, &result), finite),
                                    compare(BinOp::Ne, x),
                                ),
                            ),
                        )
                    }
                    _ => (compare(BinOp::Lt, x), constant(false, x)),
                };
                let test = match code {
                    None => or(domain, range),
                    Some(EDOM) => domain,
                    Some(ERANGE) => range,
                    Some(_) => return None,
                };
                (truth(&test) != Some(false)).then_some(test)
            }
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

/// The value of a constant argument.
fn constant_of(arg: &Expr) -> Option<f64> {
    match arg.uncast().kind {
        ExprKind::Float(value) => Some(value),
        ExprKind::Int(value) => Some(value as f64),
        _ => None,
    }
}

/// `isnan(value)`, `isinf` or `isfinite` as a `bool`, or its truth for a
/// constant.
fn test(name: &str, value: &Expr) -> Expr {
    if let Some(number) = constant_of(value) {
        let truth = match name {
            "isnan" => number.is_nan(),
            "isinf" => number.is_infinite(),
            _ => number.is_finite(),
        };
        return constant(truth, value);
    }
    let double = Type::Float { bits: 64 };
    Expr {
        ty: Type::Bool,
        kind: ExprKind::Call(
            Box::new(Expr {
                ty: Type::Function {
                    ret: Box::new(Type::Bool),
                    params: vec![double],
                    variadic: false,
                },
                kind: ExprKind::Function(format!("__builtin_{name}")),
                at: value.at,
            }),
            vec![value.clone()],
        ),
        at: value.at,
    }
}

/// `value op 0`.
fn compare(op: BinOp, value: &Expr) -> Expr {
    if let Some(number) = constant_of(value) {
        let truth = match op {
            BinOp::Eq => number == 0.0,
            BinOp::Ne => number != 0.0,
            _ => number < 0.0,
        };
        return constant(truth, value);
    }
    let zero = Expr {
        ty: value.ty.clone(),
        kind: ExprKind::Float(0.0),
        at: value.at,
    };
    Expr {
        ty: Type::Bool,
        kind: ExprKind::Binary(op, Box::new(value.clone()), Box::new(zero)),
        at: value.at,
    }
}

fn and(left: Expr, right: Expr) -> Expr {
    logical(BinOp::And, left, right)
}

fn or(left: Expr, right: Expr) -> Expr {
    logical(BinOp::Or, left, right)
}

/// Both sides are without an effect, so a constant one goes.
fn logical(op: BinOp, left: Expr, right: Expr) -> Expr {
    let decides = op == BinOp::Or;
    match (truth(&left), truth(&right)) {
        (Some(value), _) | (_, Some(value)) if value == decides => constant(decides, &left),
        (Some(_), _) => right,
        (_, Some(_)) => left,
        _ => Expr {
            ty: Type::Bool,
            at: left.at,
            kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
        },
    }
}

fn not(value: Expr) -> Expr {
    match truth(&value) {
        Some(truth) => constant(!truth, &value),
        None => Expr {
            ty: Type::Bool,
            at: value.at,
            kind: ExprKind::Unary(UnOp::Not, Box::new(value)),
        },
    }
}

fn constant(value: bool, at: &Expr) -> Expr {
    Expr {
        ty: Type::Bool,
        kind: ExprKind::Int(i128::from(value)),
        at: at.at,
    }
}
