//! `protos`, R63: C's `int (*f)()` does not say the parameters of the
//! functions it points to, and Rust's `fn` type does. The parameters are
//! those of every function that reaches such a pointer and of every call
//! through it, which must agree. The pointers are grouped by their result
//! type, a group that holds the slots which hand such a pointer on to each
//! other and possibly more; a group whose functions or calls disagree is
//! refused. A group that no function reaches and that is never called,
//! one that only holds NULL, takes no parameters.

use crate::Unsupported;
use crate::c::{Expr, ExprKind, Program, Type, each_expr_mut};
use std::collections::{HashMap, HashSet};

type R<T> = Result<T, Unsupported>;

/// The result type of a function type without a prototype.
fn unprototyped(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Function {
            ret,
            params,
            variadic: true,
        } if params.is_empty() => Some(ret),
        _ => None,
    }
}

/// The parameters of a function type with a prototype.
fn prototyped(ty: &Type) -> Option<(&Type, &[Type])> {
    match ty {
        Type::Function {
            ret,
            params,
            variadic: false,
        } => Some((ret, params)),
        _ => None,
    }
}

#[derive(Default)]
struct Group {
    /// The parameter lists of the functions that reach the group, with
    /// where each is first seen.
    params: Vec<(Vec<Type>, u32)>,
    /// The argument types of each call through a pointer of the group.
    calls: Vec<(Vec<Type>, u32)>,
}

pub fn protos(program: &mut Program) -> R<()> {
    // In the order the program first shows them, so that the refusal of
    // a program is always the same.
    let mut groups: Vec<(Type, Group)> = Vec::new();
    let functions: HashMap<String, Vec<Type>> = program
        .functions
        .iter()
        .map(|function| {
            let params = function
                .params
                .iter()
                .map(|&var| program.vars[var].ty.clone())
                .collect();
            (function.name.clone(), params)
        })
        .collect();
    each_expr(program, &mut |expr| collect(expr, &functions, &mut groups));
    let mut decided: HashMap<Type, Vec<Type>> = HashMap::new();
    for (ret, group) in &groups {
        let mut lists = group.params.iter();
        let params = match lists.next() {
            Some((params, _)) => params,
            None => {
                if let Some((_, at)) = group.calls.first() {
                    return Err(Unsupported::new(
                        "a call through a pointer to a function without a prototype that no function reaches",
                        *at,
                    ));
                }
                decided.insert(ret.clone(), Vec::new());
                continue;
            }
        };
        if let Some((_, at)) = lists.find(|(other, _)| other != params) {
            return Err(Unsupported::new(
                "a pointer to a function without a prototype that holds functions of several types",
                *at,
            ));
        }
        // C passes the arguments of such a call with the default
        // promotions, so a parameter that they change never receives what
        // the function expects; the behaviour is undefined then.
        for (args, at) in &group.calls {
            if args != params || params.iter().any(promoted) {
                return Err(Unsupported::new(
                    "a call through a pointer to a function without a prototype with arguments its functions do not take",
                    *at,
                ));
            }
        }
        decided.insert(ret.clone(), params.clone());
    }
    let retype = |ty: &mut Type| retype(ty, &decided);
    for var in &mut program.vars {
        retype(&mut var.ty);
    }
    for record in &mut program.records {
        for field in &mut record.fields {
            retype(&mut field.ty);
        }
    }
    for function in &mut program.functions {
        retype(&mut function.ret);
    }
    each_expr(program, &mut |expr| {
        if let ExprKind::SizeOf(ty) = &mut expr.kind {
            retype(ty);
        }
        retype(&mut expr.ty);
    });
    Ok(())
}

/// Whether C's default promotions change a value of the type.
fn promoted(ty: &Type) -> bool {
    match ty {
        Type::Bool | Type::Char => true,
        Type::Int { bits, .. } => *bits < 32,
        Type::Float { bits } => *bits < 64,
        _ => false,
    }
}

/// Records what the expression says of a pointer without a prototype.
fn collect(expr: &Expr, functions: &HashMap<String, Vec<Type>>, groups: &mut Vec<(Type, Group)>) {
    let at = expr.at;
    let pointed = |ty: &Type| ty.function().and_then(unprototyped).cloned();
    match &expr.kind {
        ExprKind::Call(callee, args) if callee.function().is_none() => {
            if let Some(ret) = pointed(&callee.ty) {
                let args = args.iter().map(|arg| arg.ty.clone()).collect();
                group(groups, ret).calls.push((args, at));
            }
        }
        // A function without a prototype of its own used as a value.
        ExprKind::Function(name) => {
            if let (Some(ret), Some(params)) = (pointed(&expr.ty), functions.get(name)) {
                group(groups, ret).params.push((params.clone(), at));
            }
        }
        // A pointer with a prototype converted to one without, or back.
        ExprKind::Cast(inner) => {
            let (to, from) = (expr.ty.function(), inner.ty.function());
            let (ret, other) = match (to.and_then(unprototyped), from.and_then(unprototyped)) {
                (Some(ret), None) => (ret, from.and_then(prototyped)),
                (None, Some(ret)) => (ret, to.and_then(prototyped)),
                _ => return,
            };
            if let Some((other_ret, params)) = other
                && other_ret == ret
            {
                group(groups, ret.clone())
                    .params
                    .push((params.to_vec(), at));
            }
        }
        _ => {}
    }
}

fn group(groups: &mut Vec<(Type, Group)>, ret: Type) -> &mut Group {
    let index = match groups.iter().position(|(known, _)| *known == ret) {
        Some(index) => index,
        None => {
            groups.push((ret, Group::default()));
            groups.len() - 1
        }
    };
    &mut groups[index].1
}

/// Gives every function type without a prototype the parameters decided
/// for its result type.
fn retype(ty: &mut Type, decided: &HashMap<Type, Vec<Type>>) {
    match ty {
        Type::Pointer(to) | Type::Array(to, _) => retype(to, decided),
        Type::Function {
            ret,
            params,
            variadic,
        } => {
            // One that nothing reaches and nothing calls takes none.
            if params.is_empty() && *variadic {
                *params = decided.get(&**ret).cloned().unwrap_or_default();
                *variadic = false;
            }
            for param in params.iter_mut() {
                retype(param, decided);
            }
            retype(ret, decided);
        }
        _ => {}
    }
}

/// Calls `change` on every expression of the program, the initialisers of
/// globals among them, but not on the callee of a call by name: that is the
/// function itself, which keeps its own type.
fn each_expr(program: &mut Program, change: &mut impl FnMut(&mut Expr)) {
    let mut callees = HashSet::new();
    let mut skip = |expr: &mut Expr| {
        if callees.contains(&(expr as *const Expr)) {
            return;
        }
        if let ExprKind::Call(callee, _) = &expr.kind
            && callee.function().is_some()
        {
            callees.insert(&**callee as *const Expr);
        }
        change(expr);
    };
    for function in &mut program.functions {
        each_expr_mut(&mut function.body, &mut program.vars, &mut skip);
    }
    for id in program.globals.clone() {
        if let Some(mut init) = program.vars[id].init.take() {
            init.each_mut(&mut skip);
            program.vars[id].init = Some(init);
        }
    }
}
