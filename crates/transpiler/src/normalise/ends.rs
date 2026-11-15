//! `ends`, R21 and R36: the end of a number that `strtol` and its like
//! store through their second argument becomes a result of the call. C declares `char *end` once and
//! hands `&end` to every call; here each call gets a variable of its own
//! for the end, which `cursors` then makes an index into the text that was
//! read, and the call becomes the statement `Stmt::Parse`.
//!
//! A read of `end` means the end of one call only where that call is the
//! last one before it on every path. That holds for the statements that
//! follow the call in its own block, up to the next call there. A function
//! that reads `end` anywhere else is left as it is, and refused.

use crate::c::{Expr, ExprKind, Program, Stmt, Type, Var, VarId, VarKind, any_expr, each_expr_mut};

/// The library functions whose end pointer has a rule.
pub const PARSERS: [&str; 6] = [
    "strtol", "strtoll", "strtoul", "strtoull", "strtod", "strtof",
];

/// The number of arguments of a parser: the text, the end and, for the
/// integers, the base.
fn arity(name: &str) -> usize {
    match name {
        "strtod" | "strtof" => 2,
        _ => 3,
    }
}

pub fn ends(program: &mut Program) {
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        let mut taken = program.functions[function].params.clone();
        taken.extend(&program.globals);
        declared(&body, &mut taken);
        for end in 0..program.vars.len() {
            if !taken.contains(&end) || !is_end(&body, &program.vars, end) {
                continue;
            }
            let mut attempt = Ends {
                vars: program.vars.clone(),
                end,
                taken: &taken,
                failed: false,
            };
            let mut rewritten = body.clone();
            attempt.block(&mut rewritten, None);
            // The variable may be declared where the rewrite does not
            // look, such as at the start of a `for` loop.
            let mut left = Vec::new();
            declared(&rewritten, &mut left);
            if !attempt.failed && !left.contains(&end) {
                body = rewritten;
                program.vars = attempt.vars;
            }
        }
        program.functions[function].body = body;
    }
    super::errno::errno(program);
}

/// Adds the locals that the statements declare, at any depth.
pub(super) fn declared(stmts: &[Stmt], found: &mut Vec<VarId>) {
    for stmt in stmts {
        match stmt {
            Stmt::Decl(var) => found.push(*var),
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body) => declared(body, found),
            Stmt::If(_, then, other) => {
                declared(then, found);
                declared(other, found);
            }
            Stmt::For { init, body, .. } => {
                declared(init, found);
                declared(body, found);
            }
            _ => {}
        }
    }
}

/// The call of a parser that stores its end through `&end`, if the
/// expression is one under its conversions.
fn parser_call(expr: &Expr, end: VarId) -> bool {
    match &expr.kind {
        ExprKind::Cast(inner) => parser_call(inner, end),
        ExprKind::Call(callee, args) => {
            callee
                .function()
                .is_some_and(|name| PARSERS.contains(&name) && args.len() == arity(name))
                && is_address(&args[1], end)
        }
        _ => false,
    }
}

fn is_address(expr: &Expr, var: VarId) -> bool {
    matches!(&expr.uncast().kind, ExprKind::AddrOf(place)
        if matches!(place.kind, ExprKind::Var(named) if named == var))
}

fn names(expr: &Expr, var: VarId) -> bool {
    expr.any(&mut |part| matches!(part.kind, ExprKind::Var(named) if named == var))
}

/// Whether the statement stores a value without an effect in the end, as
/// `end = NULL;` or `end = text;` do before a call.
fn is_dead_store(stmt: &Stmt, end: VarId) -> bool {
    matches!(stmt, Stmt::Expr(Expr { kind: ExprKind::Assign { op: None, target, value }, .. })
        if matches!(target.kind, ExprKind::Var(named) if named == end) && value.is_pure())
}

/// The number of such stores among the statements, at any depth.
fn dead_stores(stmts: &[Stmt], end: VarId) -> usize {
    stmts
        .iter()
        .map(|stmt| match stmt {
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body) => dead_stores(body, end),
            Stmt::If(_, then, other) => dead_stores(then, end) + dead_stores(other, end),
            Stmt::For { init, body, .. } => dead_stores(init, end) + dead_stores(body, end),
            stmt => usize::from(is_dead_store(stmt, end)),
        })
        .sum()
}

/// Whether the variable is a local `char *` that only such calls set,
/// apart from first values without an effect, which a read before the
/// first call would see and which are dropped where nothing reads them.
fn is_end(body: &[Stmt], vars: &[Var], end: VarId) -> bool {
    let var = &vars[end];
    let unset = var.init.as_ref().is_none_or(Expr::is_pure);
    if var.kind != VarKind::Local || var.ty != Type::Pointer(Box::new(Type::Char)) || !unset {
        return false;
    }
    let (mut addresses, mut calls, mut stores, mut stepped) = (0, 0, 0, false);
    any_expr(body, vars, &mut |expr| {
        match &expr.kind {
            ExprKind::AddrOf(place) if matches!(place.kind, ExprKind::Var(named) if named == end) => {
                addresses += 1
            }
            ExprKind::Call(..) if parser_call(expr, end) => calls += 1,
            ExprKind::Assign { target, .. } if matches!(target.kind, ExprKind::Var(named) if named == end) => {
                stores += 1
            }
            ExprKind::IncDec { target, .. } if matches!(target.kind, ExprKind::Var(named) if named == end) => {
                stepped = true
            }
            _ => {}
        }
        false
    });
    calls > 0 && addresses == calls && !stepped && stores == dead_stores(body, end)
}

struct Ends<'a> {
    vars: Vec<Var>,
    /// The C variable that the calls store through.
    end: VarId,
    /// The variables in whose scope the function is.
    taken: &'a [VarId],
    failed: bool,
}

impl Ends<'_> {
    /// Rewrites the statements of a block. `current` is the variable that
    /// holds the end of the last call before them, if there is one call
    /// that it must be.
    fn block(&mut self, stmts: &mut Vec<Stmt>, mut current: Option<VarId>) {
        let end = self.end;
        let mut index = 0;
        while index < stmts.len() {
            if matches!(stmts[index], Stmt::Decl(var) if var == end)
                || is_dead_store(&stmts[index], end)
            {
                // What the store left is read nowhere: a read before the
                // next call finds no `current` and fails.
                stmts.remove(index);
                current = None;
                continue;
            }
            if matches!(stmts[index], Stmt::Case(_) | Stmt::Default) {
                // A `switch` may start here, after no call at all.
                current = None;
            } else if self.without_end(&mut stmts[index], current) {
                current = None;
                // A call whose end nothing reads stays as it is.
                if self.read_after(&stmts[index + 1..])
                    && let Some((parse, alone)) = self.call(&mut stmts[index])
                {
                    if let Stmt::Parse { end, .. } = &parse {
                        current = Some(*end);
                    }
                    match alone {
                        true => stmts[index] = parse,
                        false => {
                            stmts.insert(index, parse);
                            index += 1;
                        }
                    }
                }
            } else if self.sets(&stmts[index]) {
                // The statement sets the end somewhere inside, so no one
                // call is the last before what follows it.
                self.inside(&mut stmts[index], current);
                current = None;
            } else {
                self.rename(std::slice::from_mut(&mut stmts[index]), current);
            }
            index += 1;
        }
    }

    /// Whether the statement holds a call that sets the end.
    fn sets(&self, stmt: &Stmt) -> bool {
        any_expr(std::slice::from_ref(stmt), &self.vars, &mut |expr| {
            parser_call(expr, self.end) && matches!(expr.kind, ExprKind::Call(..))
        })
    }

    /// The blocks inside a statement that sets the end. A branch starts
    /// with the end that the statement starts with, and the body of a loop
    /// with none, since it may follow itself. What is evaluated between
    /// the blocks of a loop may not name the end.
    fn inside(&mut self, stmt: &mut Stmt, current: Option<VarId>) {
        let end = self.end;
        let plain = |ends: &mut Self, exprs: &[&Expr]| {
            ends.failed |= exprs.iter().any(|expr| names(expr, end));
        };
        match stmt {
            Stmt::Block(body) => self.block(body, current),
            Stmt::If(cond, then, other) => {
                self.failed |= !cond.is_pure();
                let mut test = [Stmt::Expr(cond.clone())];
                self.rename(&mut test, current);
                if let [Stmt::Expr(test)] = test {
                    *cond = test;
                }
                self.block(then, current);
                self.block(other, current);
            }
            Stmt::While(cond, body) | Stmt::DoWhile(body, cond) | Stmt::Switch(cond, body) => {
                plain(self, &[cond]);
                self.block(body, None);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let parts: Vec<&Expr> = cond.iter().chain(step.iter()).collect();
                plain(self, &parts);
                self.failed |= init.iter().any(|stmt| self.sets(stmt));
                self.rename(init, None);
                self.block(body, None);
            }
            _ => self.failed = true,
        }
    }

    /// Names `current` where the statements read the end.
    fn rename(&mut self, stmts: &mut [Stmt], current: Option<VarId>) {
        let end = self.end;
        let mut failed = false;
        each_expr_mut(stmts, &mut self.vars, &mut |expr| {
            // A store that is not a statement of its own.
            if let ExprKind::Assign { target, .. } = &expr.kind
                && matches!(target.kind, ExprKind::Var(named) if named == end)
            {
                failed = true;
            }
            if matches!(expr.kind, ExprKind::Var(named) if named == end) {
                match current {
                    Some(current) => expr.kind = ExprKind::Var(current),
                    None => failed = true,
                }
            }
        });
        self.failed |= failed;
    }

    /// Whether a statement before the next that sets the end reads it.
    fn read_after(&self, rest: &[Stmt]) -> bool {
        let end = self.end;
        for stmt in rest {
            // Every name of the end that is not in `&end`.
            let (mut named, mut addresses) = (0, 0);
            any_expr(std::slice::from_ref(stmt), &self.vars, &mut |expr| {
                match &expr.kind {
                    ExprKind::Var(var) if *var == end => named += 1,
                    ExprKind::AddrOf(_) if is_address(expr, end) => addresses += 1,
                    _ => {}
                }
                false
            });
            // A read of `errno` asks for the call's error, which only
            // `Stmt::Parse` returns (R37).
            let errno = any_expr(std::slice::from_ref(stmt), &self.vars, &mut |expr| {
                expr.is_errno()
            });
            if named > addresses || errno {
                return true;
            }
            if self.sets(stmt) {
                return false;
            }
        }
        false
    }

    /// If the statement is `value = strtol(text, &end, 10)`, a declaration
    /// with that value or the call alone: takes `&end` out of the call
    /// and says so.
    fn without_end(&mut self, stmt: &mut Stmt, current: Option<VarId>) -> bool {
        let end = self.end;
        let Some(value) = value_of(stmt, &mut self.vars, end) else {
            return false;
        };
        if !parser_call(value, end) {
            return false;
        }
        let ExprKind::Call(_, args) = &mut under_casts(value).kind else {
            return false;
        };
        let pure = args[0].is_pure() && !args[2..].iter().any(|base| names(base, end));
        args[1].kind = ExprKind::Null;
        // The text may be the end of the call before, and is read before
        // this call sets its own.
        let mut text = [Stmt::Expr(args[0].clone())];
        self.failed |= !pure;
        self.rename(&mut text, current);
        if let ([Stmt::Expr(text)], Some(value)) = (text, value_of(stmt, &mut self.vars, end))
            && let ExprKind::Call(_, args) = &mut under_casts(value).kind
        {
            args[0] = text;
        }
        true
    }

    /// Takes the call out of a statement that `without_end` found, and
    /// returns the `Stmt::Parse` that goes before the statement, and
    /// whether that leaves nothing of the statement.
    fn call(&mut self, stmt: &mut Stmt) -> Option<(Stmt, bool)> {
        let end = self.end;
        let place = under_casts(value_of(stmt, &mut self.vars, end)?);
        let (at, call_type) = (place.at, place.ty.clone());
        let call = std::mem::replace(place, hole());
        let (name, ty) = (self.vars[end].name.clone(), self.vars[end].ty.clone());
        let this_end = self.local(name, ty, at);
        let declares = match stmt {
            Stmt::Decl(var) => Some(*var),
            _ => None,
        };
        let value = value_of(stmt, &mut self.vars, end)?;
        let whole = is_hole(value);
        let (number, alone) = match declares {
            // `long n = strtol(…)` declares `n` as the call's number.
            Some(var) if whole && self.vars[var].ty == call_type => {
                self.vars[var].init = None;
                (Some(var), true)
            }
            // The call alone has no use for its number.
            None if whole && matches!(stmt, Stmt::Expr(expr) if is_hole(expr)) => (None, true),
            _ => {
                // The declared variable's own name until it has its value,
                // or one that no variable in scope has.
                let name = match declares {
                    Some(var) => self.vars[var].name.clone(),
                    None => {
                        let mut name = "parsed".to_owned();
                        while self
                            .taken
                            .iter()
                            .any(|&other| self.vars[other].name == name)
                        {
                            name.push('_');
                        }
                        name
                    }
                };
                let number = self.local(name, call_type.clone(), at);
                *under_casts(value_of(stmt, &mut self.vars, end)?) = Expr {
                    ty: call_type,
                    kind: ExprKind::Var(number),
                    at,
                };
                (Some(number), false)
            }
        };
        let parse = Stmt::Parse {
            number,
            error: None,
            end: this_end,
            call,
            start: None,
        };
        Some((parse, alone))
    }

    fn local(&mut self, name: String, ty: Type, at: u32) -> VarId {
        self.vars.push(Var {
            name,
            ty,
            init: None,
            len: None,
            kind: VarKind::Local,
            at,
        });
        self.vars.len() - 1
    }
}

/// The value that a statement computes and keeps, where a call of a
/// parser may be taken out and run before the statement: the first value
/// of a declaration, what an assignment stores, or all of a statement
/// that is one expression.
fn value_of<'a>(stmt: &'a mut Stmt, vars: &'a mut [Var], end: VarId) -> Option<&'a mut Expr> {
    match stmt {
        Stmt::Decl(var) => vars[*var].init.as_mut(),
        Stmt::Expr(expr) => {
            // The target is evaluated after the call, which changes
            // nothing that a target without the end reads.
            let stores = matches!(&expr.kind, ExprKind::Assign { target, .. }
                if target.is_pure() && !names(target, end));
            if stores {
                let ExprKind::Assign { value, .. } = &mut expr.kind else {
                    unreachable!()
                };
                return Some(value);
            }
            Some(expr)
        }
        _ => None,
    }
}

/// The expression under the conversions around it.
fn under_casts(expr: &mut Expr) -> &mut Expr {
    if matches!(expr.kind, ExprKind::Cast(_)) {
        let ExprKind::Cast(inner) = &mut expr.kind else {
            unreachable!()
        };
        return under_casts(inner);
    }
    expr
}

/// Stands where a call was taken out of its statement.
fn hole() -> Expr {
    Expr {
        ty: Type::Void,
        kind: ExprKind::Zero,
        at: 0,
    }
}

fn is_hole(expr: &Expr) -> bool {
    matches!(expr.kind, ExprKind::Zero) && expr.ty == Type::Void
}
