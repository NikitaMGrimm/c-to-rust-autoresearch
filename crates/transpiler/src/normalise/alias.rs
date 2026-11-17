//! R61: a local pointer that only ever holds the address of one place is
//! that place. Rust rejects such a pointer as a borrow where the place
//! itself is used while the pointer is in use (`Facts::in_use`); C means
//! the place at every use, so every use names it instead.

use super::names;
use crate::c::{
    BinOp, Expr, ExprKind, Program, Stmt, Type, Var, VarId, VarKind, any_expr, each_expr_mut,
};
use crate::facts::Facts;

/// Rewrites the pointers that `Facts::in_use` finds and that only name one
/// place, and says whether it rewrote any.
pub fn alias(program: &mut Program, facts: &Facts) -> bool {
    let mut changed = false;
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        for pointer in 0..program.vars.len() {
            let var = &program.vars[pointer];
            if var.kind != VarKind::Local || !var.ty.is_pointer() || !facts.in_use_by(pointer) {
                continue;
            }
            let mut fits = |place: &Expr, root: VarId, block: &[Stmt]| {
                is_address(place, program) && !beside(block, program, pointer, root)
            };
            if let Some(place) = take_store(&mut body, program, pointer, &mut fits) {
                each_expr_mut(&mut body, &mut program.vars, &mut |expr| {
                    if matches!(expr.kind, ExprKind::Var(named) if named == pointer) {
                        *expr = Expr {
                            at: expr.at,
                            ..place.clone()
                        };
                    }
                });
                // `p[-1]` for `p = &a[2]` is `a[1]`, which is before what
                // a pointer to `a[2]` holds in Rust.
                each_expr_mut(&mut body, &mut program.vars, &mut super::offsets);
                // `*&x` is `x`: a store through the pointer is a store to
                // the variable, which the other pointers to it must see.
                each_expr_mut(&mut body, &mut program.vars, &mut |expr| {
                    if let ExprKind::Deref(inner) = &mut expr.kind
                        && let ExprKind::AddrOf(place) = &mut inner.kind
                        && place.ty == expr.ty
                    {
                        *expr = super::take(place);
                    }
                });
                changed = true;
            }
        }
        program.functions[function].body = body;
    }
    changed
}

/// Rewrites every local `T **` that only ever holds `&p`, for a local or a
/// parameter `p` of type `T *`, and is only used as `*pp` or `pp[0]`, to
/// `p` at each use. Every other use of `pp`, such as `pp + 1`, `f(pp)` or
/// a comparison, leaves it as it is.
pub fn pointers(program: &mut Program) {
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        for pointer in 0..program.vars.len() {
            let var = &program.vars[pointer];
            let Type::Pointer(to) = &var.ty else {
                continue;
            };
            // `T ***`, `void **` and a pointer to a pointer to a function
            // have no rule for the pointer they name either.
            let named = match &**to {
                Type::Pointer(named) => named,
                _ => continue,
            };
            if var.kind != VarKind::Local
                || to.function().is_some()
                || matches!(**named, Type::Pointer(_) | Type::Void)
            {
                continue;
            }
            let mut fits = |place: &Expr, root: VarId, block: &[Stmt]| {
                let vars = &program.vars;
                matches!(&place.kind, ExprKind::AddrOf(inner)
                    if matches!(inner.kind, ExprKind::Var(var) if var == root))
                    && matches!(vars[root].kind, VarKind::Local | VarKind::Param)
                    && only_through(block, program, pointer)
            };
            let Some(place) = take_store(&mut body, program, pointer, &mut fits) else {
                continue;
            };
            let ExprKind::AddrOf(named) = place.kind else {
                unreachable!()
            };
            each_expr_mut(&mut body, &mut program.vars, &mut |expr| {
                let through = match &expr.kind {
                    ExprKind::Deref(inner) | ExprKind::Index(inner, _) => {
                        matches!(inner.kind, ExprKind::Var(var) if var == pointer)
                    }
                    _ => false,
                };
                if through {
                    *expr = Expr {
                        at: expr.at,
                        ..(*named).clone()
                    };
                }
            });
        }
        program.functions[function].body = body;
    }
}

/// R69: rewrites every local `T *q = (T *)p` for a pointer `p` to bytes and
/// a number `T`, that nothing stores to again and that is only used as
/// `*q` or `q[i]`, to the cast at each use, which `lower` reads as a
/// number made from the bytes. The variables of the cast must not change
/// in the function, so that the cast means the same pointer at each use.
pub fn casts(program: &mut Program) {
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        for pointer in 0..program.vars.len() {
            let var = &program.vars[pointer];
            let Some(cast) = var.init.as_ref().filter(|cast| cast.ty == var.ty) else {
                continue;
            };
            let ExprKind::Cast(bytes) = &cast.kind else {
                continue;
            };
            if var.ty.element().and_then(Type::number_size).is_none()
                || !bytes.ty.element().is_some_and(Type::is_byte)
            {
                continue;
            }
            if var.kind != VarKind::Local
                || var.len.is_some()
                || !fixed(bytes, &body, program)
                || stores(&body, &program.vars, pointer) > 0
                || !only_read_through(&body, program, pointer)
                || !undeclare(&mut body, pointer)
            {
                continue;
            }
            let cast = cast.clone();
            each_expr_mut(&mut body, &mut program.vars, &mut |expr| {
                if matches!(expr.kind, ExprKind::Var(var) if var == pointer) {
                    *expr = Expr {
                        at: expr.at,
                        ..cast.clone()
                    };
                }
            });
        }
        program.functions[function].body = body;
    }
}

/// Whether the pointer means the same address wherever the function reads
/// it: it names only locals and parameters that the function never
/// changes, arrays of them, constants, and sums and elements of those. A
/// field, an element read as a value or a call may change between uses.
fn fixed(pointer: &Expr, body: &[Stmt], program: &Program) -> bool {
    let vars = &program.vars;
    let unchanged = |var: VarId| {
        matches!(vars[var].kind, VarKind::Local | VarKind::Param)
            && !super::assigns(body, vars, var)
            && stores(body, vars, var) == 0
            && !hides(body, vars, var)
    };
    let index = |at: &Expr| {
        at.constant(program).is_some()
            || matches!(at.uncast().kind, ExprKind::Var(var) if unchanged(var))
    };
    match &pointer.kind {
        ExprKind::Var(var) => unchanged(*var),
        ExprKind::Cast(inner) => fixed(inner, body, program),
        ExprKind::Decay(array) => matches!(array.kind, ExprKind::Var(var) if unchanged(var)),
        ExprKind::Binary(BinOp::Add, left, right) => match left.ty.is_pointer() {
            true => fixed(left, body, program) && index(right),
            false => fixed(right, body, program) && index(left),
        },
        ExprKind::AddrOf(place) => match &place.kind {
            ExprKind::Index(base, at) => fixed(base, body, program) && index(at),
            _ => false,
        },
        _ => false,
    }
}

/// Whether the statements use the pointer only as `*pointer` and
/// `pointer[i]`.
fn only_read_through(stmts: &[Stmt], program: &Program, pointer: VarId) -> bool {
    let is_pointer = |expr: &Expr| matches!(expr.kind, ExprKind::Var(var) if var == pointer);
    let (mut uses, mut through) = (0, 0);
    any_expr(stmts, &program.vars, &mut |expr| {
        match &expr.kind {
            ExprKind::Var(var) if *var == pointer => uses += 1,
            ExprKind::Deref(inner) | ExprKind::Index(inner, _) if is_pointer(inner) => through += 1,
            _ => {}
        }
        false
    });
    uses == through
}

/// Takes the declaration of the variable out of the statements, and says
/// whether it found it.
fn undeclare(stmts: &mut Vec<Stmt>, var: VarId) -> bool {
    if let Some(at) = stmts
        .iter()
        .position(|stmt| matches!(stmt, Stmt::Decl(declared) if *declared == var))
    {
        stmts.remove(at);
        return true;
    }
    stmts.iter_mut().any(|stmt| match stmt {
        Stmt::Block(body)
        | Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::Switch(_, body)
        | Stmt::For { body, .. } => undeclare(body, var),
        Stmt::If(_, then, other) => undeclare(then, var) || undeclare(other, var),
        _ => false,
    })
}

/// Whether the statements use the pointer only as `*pointer` and
/// `pointer[0]`, apart from its one store.
fn only_through(stmts: &[Stmt], program: &Program, pointer: VarId) -> bool {
    let is_pointer = |expr: &Expr| matches!(expr.kind, ExprKind::Var(var) if var == pointer);
    let (mut uses, mut through, mut stored) = (0, 0, 0);
    any_expr(stmts, &program.vars, &mut |expr| {
        match &expr.kind {
            ExprKind::Var(var) if *var == pointer => uses += 1,
            ExprKind::Deref(inner) if is_pointer(inner) => through += 1,
            ExprKind::Index(inner, at) if is_pointer(inner) && at.constant(program) == Some(0) => {
                through += 1;
            }
            ExprKind::Assign {
                op: None, target, ..
            } if is_pointer(target) => stored += 1,
            _ => {}
        }
        false
    });
    uses == through + stored
}

/// The address that the pointer only ever holds, with its declaration and
/// its one store taken out of the statements, or nothing where it may hold
/// another.
fn take_store(
    stmts: &mut Vec<Stmt>,
    program: &Program,
    pointer: VarId,
    // Whether the address fits, given the place it names and the block
    // that declares the pointer.
    fits: &mut dyn FnMut(&Expr, VarId, &[Stmt]) -> bool,
) -> Option<Expr> {
    let vars = &program.vars;
    let Some(decl) = stmts
        .iter()
        .position(|stmt| matches!(stmt, Stmt::Decl(var) if *var == pointer))
    else {
        return stmts.iter_mut().find_map(|stmt| match stmt {
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body)
            | Stmt::For { body, .. } => take_store(body, program, pointer, fits),
            Stmt::If(_, then, other) => take_store(then, program, pointer, fits)
                .or_else(|| take_store(other, program, pointer, fits)),
            _ => None,
        });
    };
    let declared = &vars[pointer];
    if declared.len.is_some() {
        return None;
    }
    let stores = stores(stmts, vars, pointer);
    let (place, store) = match &declared.init {
        Some(init) if stores == 0 => (init.clone(), None),
        Some(_) => return None,
        None if stores == 1 => {
            // The first statement after the declaration that names the
            // pointer is its store, so that nothing reads it before.
            let first = (decl + 1..stmts.len()).find(|&at| names(&stmts[at], vars, pointer))?;
            let Stmt::Expr(Expr {
                kind:
                    ExprKind::Assign {
                        op: None,
                        target,
                        value,
                    },
                ..
            }) = &stmts[first]
            else {
                return None;
            };
            if !matches!(target.kind, ExprKind::Var(var) if var == pointer) {
                return None;
            }
            ((**value).clone(), Some(first))
        }
        None => return None,
    };
    let place = element(place, program);
    let root = match &place.kind {
        ExprKind::AddrOf(inner) | ExprKind::Decay(inner) => inner.root_var(),
        _ => None,
    };
    let root = root.filter(|_| place.ty == declared.ty)?;
    // The Rust names follow the C names, so a variable that hides the
    // place where the pointer is used would be named in its place.
    if hides(&stmts[decl + 1..], vars, root) || !fits(&place, root, stmts) {
        return None;
    }
    if let Some(store) = store {
        stmts.remove(store);
    }
    stmts.remove(decl);
    Some(place)
}

/// `a + 2` for an array `a` and a constant that is not negative is
/// `&a[2]`, the address of an element.
fn element(place: Expr, program: &Program) -> Expr {
    let ExprKind::Binary(BinOp::Add, array, index) = &place.kind else {
        return place;
    };
    let Some(to) = place.ty.element() else {
        return place;
    };
    if !matches!(array.kind, ExprKind::Decay(_))
        || array.ty != place.ty
        || index.constant(program).is_none_or(|index| index < 0)
    {
        return place;
    }
    let element = Expr {
        ty: to.clone(),
        kind: ExprKind::Index(array.clone(), index.clone()),
        at: place.at,
    };
    Expr {
        ty: place.ty.clone(),
        kind: ExprKind::AddrOf(Box::new(element)),
        at: place.at,
    }
}

/// Whether a call in the statements takes the pointer, as it is or as
/// `&*p` or `&p->f`, or the address of the place, and reads the place or
/// the pointer in another argument: it would borrow the place and read it
/// at once.
fn beside(stmts: &[Stmt], program: &Program, pointer: VarId, root: VarId) -> bool {
    let is = |expr: &Expr, var: VarId| matches!(expr.uncast().kind, ExprKind::Var(named) if named == var);
    any_expr(stmts, &program.vars, &mut |expr| match &expr.kind {
        // The library's functions that print only read their arguments.
        ExprKind::Call(callee, args)
            if callee.function().is_none_or(|name| {
                program.function(name).is_some()
                    || !["printf", "fprintf", "puts", "fputs"].contains(&name)
            }) =>
        {
            args.iter().enumerate().any(|(at, arg)| {
                arg.ty.is_pointer()
                    && arg.any(&mut |part| is(part, pointer) || is(part, root))
                    && args.iter().enumerate().any(|(other, arg)| {
                        other != at && arg.any(&mut |part| is(part, root) || is(part, pointer))
                    })
            })
        }
        _ => false,
    })
}

/// Whether the statements declare another variable with the name of `var`.
pub(super) fn hides(stmts: &[Stmt], vars: &[Var], var: VarId) -> bool {
    let other = |declared: VarId| declared != var && vars[declared].name == vars[var].name;
    stmts.iter().any(|stmt| match stmt {
        Stmt::Decl(declared) => other(*declared),
        Stmt::Block(body)
        | Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::Switch(_, body) => hides(body, vars, var),
        Stmt::For { init, body, .. } => hides(init, vars, var) || hides(body, vars, var),
        Stmt::If(_, then, other) => hides(then, vars, var) || hides(other, vars, var),
        _ => false,
    }) || any_expr(stmts, vars, &mut |expr| match &expr.kind {
        ExprKind::Init { var: declared, .. } => other(*declared),
        _ => false,
    })
}

/// How often the statements store to the pointer or take its address.
fn stores(stmts: &[Stmt], vars: &[Var], pointer: VarId) -> usize {
    let is_pointer = |expr: &Expr| matches!(expr.kind, ExprKind::Var(var) if var == pointer);
    let mut count = 0;
    any_expr(stmts, vars, &mut |expr| {
        count += match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => {
                usize::from(is_pointer(target))
            }
            ExprKind::AddrOf(inner) => usize::from(is_pointer(inner)),
            ExprKind::Init { var, .. } => usize::from(*var == pointer),
            _ => 0,
        };
        false
    });
    count
}

/// Whether the value is the address of a place that stays the same place
/// while the pointer is used: a variable that is no pointer, a field of
/// one, or an element at a constant index, or such an array as a pointer.
fn is_address(value: &Expr, program: &Program) -> bool {
    match &value.kind {
        ExprKind::AddrOf(place) | ExprKind::Decay(place) => is_fixed(place, program),
        _ => false,
    }
}

fn is_fixed(place: &Expr, program: &Program) -> bool {
    match &place.kind {
        ExprKind::Var(var) => !program.vars[*var].ty.is_pointer(),
        ExprKind::Member {
            base, arrow: false, ..
        } => is_fixed(base, program),
        ExprKind::Index(base, index) => match &base.kind {
            ExprKind::Decay(array) => index.constant(program).is_some() && is_fixed(array, program),
            _ => false,
        },
        _ => false,
    }
}
