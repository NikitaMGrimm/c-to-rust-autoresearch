//! `found`, R40: a local pointer that only ever holds what a search of the
//! library found, `strchr` and its like, and that is only tested for NULL,
//! is whether the search found something and where: `Type::Found`, an
//! `Option<usize>` in Rust. A local that is used in any other way stays a
//! pointer, and the analysis refuses the library's pointer in it.

use super::cursors::{address, as_index, element, index_literal};
use crate::c::{BinOp, Expr, ExprKind, Program, Stmt, Type, UnOp, Var, VarId, VarKind};

/// The library functions that return a place in their first argument, or
/// NULL.
pub const SEARCHES: [&str; 5] = ["strchr", "strrchr", "memchr", "strstr", "strpbrk"];

/// Whether the expression is a call of one of the library's searches.
pub fn is_search(program: &Program, expr: &Expr) -> bool {
    matches!(&expr.uncast().kind, ExprKind::Call(callee, _)
    if callee.function().is_some_and(|name| {
        (SEARCHES.contains(&name) && program.function(name).is_none())
            || program.searches.iter().any(|search| search == name)
    }))
}

pub fn found(program: &mut Program) {
    // R51: the program's functions that may be searches themselves. One
    // whose result is used as a pointer somewhere is none, which shows
    // only when the locals have their types, so the stage runs again
    // without it.
    program.searches.clear();
    loop {
        // A function that returns what another of them found is one too.
        let searches = candidates(program);
        if searches == program.searches {
            break;
        }
        program.searches = searches;
    }
    loop {
        let vars = program.vars.clone();
        let functions: Vec<(Type, Vec<Stmt>)> = program
            .functions
            .iter()
            .map(|function| (function.ret.clone(), function.body.clone()))
            .collect();
        run(program);
        let strays = strays(program);
        if strays.is_empty() {
            return;
        }
        program.vars = vars;
        for (function, (ret, body)) in program.functions.iter_mut().zip(functions) {
            (function.ret, function.body) = (ret, body);
        }
        program.searches.retain(|name| !strays.contains(name));
    }
}

/// The value of a `return` that is NULL, or a search in the variable.
fn finds_in(program: &Program, value: &Expr, param: VarId) -> Option<bool> {
    let value = value.uncast();
    match &value.kind {
        ExprKind::Null => Some(false),
        ExprKind::Call(_, args) if is_search(program, value) => {
            matches!(args.first()?.uncast().kind, ExprKind::Var(var) if var == param)
                .then_some(true)
        }
        _ => None,
    }
}

/// R51: the functions that return a place in their first parameter or
/// NULL: every `return` gives NULL or what a search of the library in
/// that parameter found, and nothing changes the parameter, so that the
/// place counts from where the caller's argument points.
fn candidates(program: &Program) -> Vec<String> {
    let mut names = Vec::new();
    for function in &program.functions {
        let Some(&param) = function.params.first() else {
            continue;
        };
        if !function.ret.is_pointer() || !program.vars[param].ty.is_pointer() {
            continue;
        }
        let is_param = |expr: &Expr| matches!(expr.kind, ExprKind::Var(var) if var == param);
        let changed = crate::c::any_expr(
            &function.body,
            &program.vars,
            &mut |expr| match &expr.kind {
                ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => {
                    is_param(target)
                }
                ExprKind::AddrOf(inner) => is_param(inner),
                _ => false,
            },
        );
        let (mut all, mut any) = (true, false);
        each_stmt(&function.body, &mut |stmt| {
            if let Stmt::Return(Some(value)) = stmt {
                match finds_in(program, value, param) {
                    Some(search) => any |= search,
                    None => all = false,
                }
            }
        });
        if !changed && all && any {
            names.push(function.name.clone());
        }
    }
    names
}

/// The searches of the program whose result is used otherwise than as the
/// value of a local that is a place, or as the result of another search.
fn strays(program: &Program) -> Vec<String> {
    let mut strays = Vec::new();
    for name in &program.searches {
        let calls = |expr: &Expr| matches!(&expr.uncast().kind, ExprKind::Call(callee, _) if callee.function() == Some(name));
        let (mut named, mut placed) = (0, 0);
        for function in &program.functions {
            crate::c::any_expr(&function.body, &program.vars, &mut |expr| {
                named += usize::from(expr.function() == Some(name));
                false
            });
            each_stmt(&function.body, &mut |stmt| {
                let value = match stmt {
                    Stmt::Decl(var) if program.vars[*var].ty == Type::Found => {
                        program.vars[*var].init.as_ref()
                    }
                    Stmt::Expr(Expr {
                        kind:
                            ExprKind::Assign {
                                op: None,
                                target,
                                value,
                            },
                        ..
                    }) if target.ty == Type::Found => Some(&**value),
                    Stmt::Return(Some(value)) if value.ty == Type::Found => Some(value),
                    _ => None,
                };
                placed += usize::from(value.is_some_and(calls));
            });
        }
        if named != placed {
            strays.push(name.clone());
        }
    }
    strays
}

fn each_stmt(stmts: &[Stmt], visit: &mut impl FnMut(&Stmt)) {
    for stmt in stmts {
        visit(stmt);
        match stmt {
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body) => each_stmt(body, visit),
            Stmt::If(_, then, other) | Stmt::IfSome { then, other, .. } => {
                each_stmt(then, visit);
                each_stmt(other, visit);
            }
            Stmt::For { init, body, .. } => {
                each_stmt(init, visit);
                each_stmt(body, visit);
            }
            Stmt::LetSome { otherwise, .. } => each_stmt(otherwise, visit),
            _ => {}
        }
    }
}

/// Gives each `return` of a search of the program the type of a place.
fn returns(stmts: &mut [Stmt]) {
    for stmt in stmts {
        match stmt {
            Stmt::Return(Some(value)) => {
                *value = value.uncast().clone();
                value.ty = Type::Found;
            }
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body)
            | Stmt::For { body, .. } => returns(body),
            Stmt::If(_, then, other) | Stmt::IfSome { then, other, .. } => {
                returns(then);
                returns(other);
            }
            Stmt::LetSome { otherwise, .. } => returns(otherwise),
            _ => {}
        }
    }
}

fn run(program: &mut Program) {
    for function in 0..program.functions.len() {
        if program.searches.contains(&program.functions[function].name) {
            program.functions[function].ret = Type::Found;
            returns(&mut program.functions[function].body);
        }
    }
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        let stored = stored(&body, &program.vars);
        // A name for what a search found that no variable has where the
        // function's loops are.
        let mut locals = program.functions[function].params.clone();
        declared(&body, &mut locals);
        locals.extend(&program.globals);
        let mut name = String::from("at");
        while locals.iter().any(|var| program.vars[*var].name == name) {
            name.push('_');
        }
        onward(&mut body, program, &stored, &name);
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
                moving: false,
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
            let Some((base, moving)) = base_of(program, var, &body) else {
                continue;
            };
            let check = Check {
                base: Some(base),
                moving,
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
    /// Whether the program stores other pointers in `base`: the place is
    /// then used only under a test that follows its search at once.
    moving: bool,
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
        stmts.iter().enumerate().all(|(index, stmt)| {
            let tests = matches!(stmt, Stmt::If(cond, ..) if self.tested(cond).is_some());
            self.stmt(stmt)
                && (!self.moving || !tests || index > 0 && self.searches(&stmts[index - 1]))
        })
    }

    /// Whether the statement stores a search in the variable.
    fn searches(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Decl(var) if *var == self.var => {
                let init = self.program.vars[*var].init.as_ref();
                init.is_some_and(|init| is_search(self.program, init))
            }
            Stmt::Expr(Expr {
                kind:
                    ExprKind::Assign {
                        op: None,
                        target,
                        value,
                    },
                ..
            }) => self.is_var(target) && is_search(self.program, value),
            _ => false,
        }
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
    /// it only as a place: nothing stores to it or to the pointer it counts
    /// from, or takes its address.
    fn placed(&self, stmts: &[Stmt]) -> bool {
        let is_base =
            |expr: &Expr| matches!(expr.kind, ExprKind::Var(var) if Some(var) == self.base);
        !crate::c::any_expr(stmts, &self.program.vars, &mut |expr| match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => {
                self.is_var(target) || is_base(target)
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
///
/// A local pointer that the program does store to is the base as well, and
/// the second result says so: the place is then good only until the next
/// store.
fn base_of(program: &Program, var: VarId, body: &[Stmt]) -> Option<(VarId, bool)> {
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
        _ => false,
    });
    let escapes = crate::c::any_expr(
        body,
        &program.vars,
        &mut |expr| matches!(&expr.kind, ExprKind::AddrOf(inner) if is_base(inner)),
    );
    let moving = match program.vars[base].ty {
        Type::Array(..) => false,
        Type::Pointer(_) if !escapes && !program.vars[base].kind.is_global() => changed,
        _ => return None,
    };
    (base != var && bases.iter().all(|other| *other == Some(base))).then_some((base, moving))
}

/// R47: writes each `if` that tests the variable as a `Stmt::IfSome`, and
/// the variable in the branch where it was found as an index into `base`.
fn narrow(stmts: &mut [Stmt], program: &mut Program, var: VarId, base: &Expr) {
    for stmt in stmts {
        let check = Check {
            program,
            var,
            base: None,
            moving: false,
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
        // R52: a step of the place is a step of its index.
        match &mut expr.kind {
            ExprKind::IncDec { target, .. } if is_var(target) => {
                target.ty = Type::Index;
                return;
            }
            ExprKind::Assign {
                op: Some((_, ty)),
                target,
                value,
            } if is_var(target) => {
                (target.ty, *ty) = (Type::Index, Type::Index);
                **value = as_index(value, records);
                return;
            }
            _ => {}
        }
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

/// The variables that the statements store to or take the address of.
fn stored(stmts: &[Stmt], vars: &[Var]) -> Vec<VarId> {
    let mut stored = Vec::new();
    crate::c::any_expr(stmts, vars, &mut |expr| {
        if let ExprKind::Assign { target, .. }
        | ExprKind::IncDec { target, .. }
        | ExprKind::AddrOf(target) = &expr.kind
            && let ExprKind::Var(var) = target.kind
        {
            stored.push(var);
        }
        false
    });
    stored
}

/// R52: a loop whose condition searches on from what it found,
/// `for (s = text; (s = strchr(s, c)); s++)`, counts the place in an index
/// beside the array: the loop goes on while the rest of the array from the
/// index holds what is searched for, and the index moves to it.
fn onward(stmts: &mut [Stmt], program: &mut Program, stored: &[VarId], name: &str) {
    for index in 0..stmts.len() {
        match &mut stmts[index] {
            Stmt::Block(body) | Stmt::DoWhile(body, _) | Stmt::Switch(_, body) => {
                onward(body, program, stored, name)
            }
            Stmt::If(_, then, other) | Stmt::IfSome { then, other, .. } => {
                onward(then, program, stored, name);
                onward(other, program, stored, name);
            }
            Stmt::LetSome { otherwise, .. } => onward(otherwise, program, stored, name),
            Stmt::While(_, body) | Stmt::For { body, .. } => onward(body, program, stored, name),
            _ => {}
        }
        let (before, rest) = stmts.split_at_mut(index);
        let (stmt, after) = rest.split_first_mut().unwrap();
        let var = match (&*stmt, before.last()) {
            (Stmt::For { init, .. }, _) if matches!(init.as_slice(), [Stmt::Decl(_)]) => {
                match init[0] {
                    Stmt::Decl(var) => var,
                    _ => continue,
                }
            }
            // The pointer is NULL after the loop, and nothing may read it.
            (Stmt::While(..), Some(Stmt::Decl(var)))
                if !after
                    .iter()
                    .any(|stmt| super::names(stmt, &program.vars, *var)) =>
            {
                *var
            }
            _ => continue,
        };
        let (cond, step, body) = match stmt {
            Stmt::For {
                cond: Some(cond),
                step,
                body,
                ..
            } => (cond, step.take(), body),
            Stmt::While(cond, body) => (cond, None, body),
            _ => continue,
        };
        let mut moved: Vec<Stmt> = step.iter().cloned().map(Stmt::Expr).collect();
        let Some((base, search, skip)) = goes_on(program, var, cond, &moved, body, stored) else {
            if let Stmt::For { step: place, .. } = stmt {
                *place = step;
            }
            continue;
        };
        let at = cond.at;
        let found = program.vars.len();
        program.vars.push(Var {
            name: name.to_string(),
            ty: Type::Index,
            init: None,
            len: None,
            kind: VarKind::Local,
            at,
        });
        let index = |var: VarId| Expr {
            ty: Type::Index,
            kind: ExprKind::Var(var),
            at,
        };
        let mut body = std::mem::take(body);
        body.insert(
            0,
            Stmt::LetSome {
                var: found,
                source: Expr {
                    ty: Type::Found,
                    ..search
                },
                otherwise: vec![Stmt::Break],
            },
        );
        // The uses of the pointer are places in the array, as in R47.
        let mut retype = |expr: &mut Expr| {
            if matches!(expr.kind, ExprKind::Var(named) if named == var) {
                expr.ty = Type::Found;
            }
        };
        crate::c::each_expr_mut(&mut body, &mut program.vars, &mut retype);
        crate::c::each_expr_mut(&mut moved, &mut program.vars, &mut retype);
        place(&mut body, program, var, &base);
        place(&mut moved, program, var, &base);
        let by = match skip {
            Some(skip) => Expr {
                ty: Type::Index,
                kind: ExprKind::Binary(
                    BinOp::Add,
                    Box::new(as_index(&skip, program)),
                    Box::new(index(found)),
                ),
                at,
            },
            None => index(found),
        };
        body.insert(
            1,
            Stmt::Expr(Expr {
                ty: Type::Index,
                kind: ExprKind::Assign {
                    op: Some((BinOp::Add, Type::Index)),
                    target: Box::new(index(var)),
                    value: Box::new(by),
                },
                at,
            }),
        );
        program.vars[var].ty = Type::Index;
        program.vars[var].init = Some(index_literal(0, at));
        let step = moved.pop().map(|stmt| match stmt {
            Stmt::Expr(step) => step,
            _ => unreachable!("the step is an expression"),
        });
        let init = match stmt {
            Stmt::For { init, .. } => std::mem::take(init),
            _ => Vec::new(),
        };
        *stmt = Stmt::For {
            init,
            cond: None,
            step,
            body,
        };
    }
}

/// The array that the loop's variable walks, the search in its condition
/// and what the search skips of the array before it, if the loop is one
/// of R52: the variable starts at the array, the condition stores in it
/// what a search of the library from the variable finds, and the step and
/// the body move it only by whole statements.
fn goes_on(
    program: &Program,
    var: VarId,
    cond: &Expr,
    step: &[Stmt],
    body: &[Stmt],
    stored: &[VarId],
) -> Option<(Expr, Expr, Option<Expr>)> {
    let declared = &program.vars[var];
    let is_var = |expr: &Expr| matches!(expr.uncast().kind, ExprKind::Var(named) if named == var);
    let mentions = |expr: &Expr| expr.any(&mut |part| is_var(part));
    if declared.kind != VarKind::Local || !declared.ty.is_pointer() {
        return None;
    }
    // Where it starts: an array, or a pointer that the function leaves
    // as it is.
    let base = declared.init.as_ref()?.uncast();
    let fixed = match &base.kind {
        ExprKind::Decay(array) => matches!(array.kind, ExprKind::Var(_)),
        ExprKind::Var(pointer) => {
            !program.vars[*pointer].kind.is_global() && !stored.contains(pointer)
        }
        _ => false,
    };
    if !fixed || !base.ty.is_pointer() {
        return None;
    }
    // `s = search(s, …)`, also as `(s = …) != NULL`.
    let mut cond = cond;
    loop {
        cond = match &cond.kind {
            ExprKind::Cast(inner) => inner,
            ExprKind::Binary(BinOp::Ne, left, right)
                if matches!(right.uncast().kind, ExprKind::Null) =>
            {
                left
            }
            _ => break,
        };
    }
    let ExprKind::Assign {
        op: None,
        target,
        value,
    } = &cond.kind
    else {
        return None;
    };
    let search = value.uncast();
    let ExprKind::Call(callee, args) = &search.kind else {
        return None;
    };
    let library = callee
        .function()
        .is_some_and(|name| SEARCHES.contains(&name) && program.function(name).is_none());
    let (from, others) = args.split_first()?;
    let skip = match &from.uncast().kind {
        _ if is_var(from) => None,
        ExprKind::Binary(BinOp::Add, left, right)
            if is_var(left) && right.ty.is_integer() && right.is_pure() && !mentions(right) =>
        {
            Some((**right).clone())
        }
        _ => return None,
    };
    if !is_var(target) || !library || others.iter().any(mentions) {
        return None;
    }
    // The pointer moves by a statement of its own, `s++;` or `s += n;`,
    // whose value nothing reads.
    let moves = |expr: &Expr| match &expr.kind {
        ExprKind::IncDec { target, .. } => is_var(target),
        ExprKind::Assign {
            op: Some((BinOp::Add | BinOp::Sub, _)),
            target,
            value,
        } => is_var(target) && value.ty.is_integer() && !mentions(value),
        _ => false,
    };
    let touched = |stmts: &[Stmt]| {
        let mut count = 0;
        crate::c::any_expr(stmts, &program.vars, &mut |expr| {
            count += usize::from(match &expr.kind {
                ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => is_var(target),
                ExprKind::AddrOf(inner) => is_var(inner),
                _ => false,
            });
            false
        });
        count
    };
    let mut whole = 0;
    each_stmt(body, &mut |stmt| {
        whole += usize::from(matches!(stmt, Stmt::Expr(expr) if moves(expr)));
    });
    whole += step
        .iter()
        .filter(|stmt| matches!(stmt, Stmt::Expr(expr) if moves(expr)))
        .count();
    (touched(body) + touched(step) == whole).then(|| (base.clone(), search.clone(), skip))
}
