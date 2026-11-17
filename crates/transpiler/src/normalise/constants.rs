//! `constants`, R72: an integer compared with itself or taken from itself is a
//! constant, `y < y` is 0 and `b - b` is 0, and a Rust programmer writes
//! the constant: the compiler's lints refuse the comparison. The constant
//! then stands inside other expressions, where the lints refuse it again
//! (`0 * y`, `false && c`), so it is folded upward with C's meaning as far
//! as a constant goes, and left out where the other side is the value
//! (`1 * z`, `z + 0`). A constant that the program wrote itself is only
//! folded where it meets one that this rewrite made.

use super::{leaves, take};
use crate::Unsupported;
use crate::c::{BinOp, Expr, ExprKind, Program, Stmt, Type, UnOp, each_expr_mut};
use crate::lower::same_value;

type R<T> = Result<T, Unsupported>;

pub fn constants(program: &mut Program) -> R<()> {
    let mut refusal = None;
    let Program {
        functions, vars, ..
    } = program;
    for function in functions {
        if let Err(error) = decided(&function.body) {
            refusal.get_or_insert(error);
        }
        each_expr_mut(&mut function.body, vars, &mut |expr| {
            if let Err(error) = made(expr) {
                refusal.get_or_insert(error);
            }
        });
    }
    refusal.map_or(Ok(()), Err)
}

/// Refuses an `if` whose test becomes a constant where the branch that
/// runs always leaves: what follows it never runs, and the compiler's
/// lints refuse a loop that it ends in its first round.
fn decided(stmts: &[Stmt]) -> R<()> {
    for stmt in stmts {
        match stmt {
            Stmt::If(cond, then, other) => {
                let mut folded = cond.clone();
                if made(&mut folded)?
                    && let Some(value) = constant(&folded)
                    && leaves(if value != 0 { then } else { other })
                {
                    return Err(Unsupported::new(
                        "a test that a value compared with itself decides, before a jump: no rule yet",
                        cond.at,
                    ));
                }
                decided(then)?;
                decided(other)?;
            }
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body)
            | Stmt::For { body, .. } => decided(body)?,
            Stmt::IfSome { then, other, .. } => {
                decided(then)?;
                decided(other)?;
            }
            Stmt::LetSome { otherwise, .. } => decided(otherwise)?,
            _ => {}
        }
    }
    Ok(())
}

/// Folds `expr` and says whether it is now a constant that this rewrite
/// made.
fn made(expr: &mut Expr) -> R<bool> {
    let mut flags = Vec::new();
    for child in expr.children_mut() {
        flags.push(made(child)?);
    }
    // `*&x`, which `alias` writes for a pointer that is its variable, is
    // `x`, and is compared as `x`.
    if let ExprKind::Deref(inner) = &mut expr.kind
        && let ExprKind::AddrOf(place) = &mut inner.kind
        && place.ty == expr.ty
    {
        *expr = take(place);
    }
    let (ty, at) = (expr.ty.clone(), expr.at);
    if let ExprKind::Binary(op, left, right) = &expr.kind
        && left.ty.is_integer()
        && right.ty.is_integer()
        && same_value(left, right)
        // Two constants are compared by the rules for them.
        && left.any(&mut |part| matches!(part.kind, ExprKind::Var(_)))
        && let Some(value) = match op {
            BinOp::Eq | BinOp::Le | BinOp::Ge => Some(1),
            BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Sub | BinOp::BitXor => Some(0),
            _ => None,
        }
    {
        *expr = literal(value, ty, at);
        return Ok(true);
    }
    if !flags.contains(&true) {
        return Ok(false);
    }
    let value = match &mut expr.kind {
        ExprKind::Binary(op @ (BinOp::And | BinOp::Or), left, right) => {
            let op = *op;
            // The value that decides the operator whatever its other side.
            let decides = |value: i128| (value == 0) == (op == BinOp::And);
            let (first, second) = (constant(left), constant(right));
            match (first, second) {
                (Some(first), _) if decides(first) => Some(i128::from(op == BinOp::Or)),
                (Some(_), Some(second)) => Some(i128::from(second != 0)),
                (Some(_), None) => {
                    *expr = truth(take(right), ty, at)?;
                    return Ok(false);
                }
                // C evaluates the left side also where the right decides.
                (None, Some(_)) if !pure(left) => {
                    return Err(Unsupported::new(
                        "a constant beside an operand with an effect: no rule yet",
                        at,
                    ));
                }
                (None, Some(second)) if decides(second) => Some(i128::from(op == BinOp::Or)),
                (None, Some(_)) => {
                    *expr = truth(take(left), ty, at)?;
                    return Ok(false);
                }
                (None, None) => None,
            }
        }
        ExprKind::Binary(op, left, right) => match (*op, constant(left), constant(right)) {
            (op, Some(left), Some(right)) => compute(op, left, right, &ty),
            (BinOp::Mul | BinOp::BitAnd, Some(0), None) if pure(right) => Some(0),
            (BinOp::Mul | BinOp::BitAnd, None, Some(0)) if pure(left) => Some(0),
            (BinOp::Rem, None, Some(1)) if pure(left) => Some(0),
            // The other side is the value, where it has the type.
            (BinOp::Mul, Some(1), None)
            | (BinOp::Add | BinOp::BitOr | BinOp::BitXor, Some(0), None)
                if right.ty == ty =>
            {
                *expr = take(right);
                return Ok(false);
            }
            (BinOp::Mul | BinOp::Div, None, Some(1))
            | (
                BinOp::Add | BinOp::Sub | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr,
                None,
                Some(0),
            ) if left.ty == ty => {
                *expr = take(left);
                return Ok(false);
            }
            _ => None,
        },
        ExprKind::Unary(op, inner) => constant(inner).and_then(|value| match op {
            UnOp::Not => Some(i128::from(value == 0)),
            UnOp::Neg => fit(-value, &ty, false),
            UnOp::BitNot => fit(!value, &ty, true),
        }),
        ExprKind::Cast(_) => constant(expr),
        ExprKind::Cond(cond, then, other) => match constant(cond) {
            Some(cond) if flags[0] => {
                let (side, made) = match cond != 0 {
                    true => (take(then), flags[1]),
                    false => (take(other), flags[2]),
                };
                *expr = side;
                return Ok(made);
            }
            _ => None,
        },
        _ => None,
    };
    Ok(match value {
        Some(value) => {
            *expr = literal(value, ty, at);
            true
        }
        None => false,
    })
}

/// The value of a constant as the rewrite reads it: a literal, its
/// negative, or one converted to an integer type.
fn constant(expr: &Expr) -> Option<i128> {
    match &expr.kind {
        ExprKind::Int(value) => Some(*value),
        ExprKind::Char(value) => Some(i128::from(*value)),
        ExprKind::Unary(UnOp::Neg, inner) => match inner.kind {
            ExprKind::Int(value) => fit(-value, &expr.ty, false),
            _ => None,
        },
        ExprKind::Cast(inner) if expr.ty == Type::Bool => {
            constant(inner).map(|value| i128::from(value != 0))
        }
        ExprKind::Cast(inner) if inner.ty.is_integer() => {
            constant(inner).and_then(|value| fit(value, &expr.ty, true))
        }
        _ => None,
    }
}

/// `value` in the integer type `ty`. A conversion wraps, as it does on
/// the host; arithmetic that leaves a signed type is undefined in C and is
/// left as the program wrote it.
fn fit(value: i128, ty: &Type, converts: bool) -> Option<i128> {
    let (bits, signed) = match ty {
        Type::Index => return None,
        ty => ty.int()?,
    };
    let (bits, signed) = if *ty == Type::Bool {
        (8, false)
    } else {
        (bits, signed)
    };
    let size = 1_i128 << bits;
    let (least, greatest) = match signed {
        true => (-(size / 2), size / 2 - 1),
        false => (0, size - 1),
    };
    if (least..=greatest).contains(&value) {
        return Some(value);
    }
    if signed && !converts {
        return None;
    }
    let wrapped = value.rem_euclid(size);
    Some(if wrapped > greatest {
        wrapped - size
    } else {
        wrapped
    })
}

/// What C computes for `left op right` in the type `ty` of the
/// expression, or `None` where it leaves the result open.
fn compute(op: BinOp, left: i128, right: i128, ty: &Type) -> Option<i128> {
    let (bits, _) = ty.int()?;
    let shift = || {
        u32::try_from(right)
            .ok()
            .filter(|&by| by < u32::from(bits) && left >= 0)
    };
    let value = match op {
        BinOp::Add => left.checked_add(right)?,
        BinOp::Sub => left.checked_sub(right)?,
        BinOp::Mul => left.checked_mul(right)?,
        BinOp::Div if right != 0 => left / right,
        BinOp::Rem if right != 0 => left % right,
        BinOp::Div | BinOp::Rem => return None,
        BinOp::Shl => left.checked_shl(shift()?)?,
        BinOp::Shr => left >> shift()?,
        BinOp::BitAnd => left & right,
        BinOp::BitOr => left | right,
        BinOp::BitXor => left ^ right,
        BinOp::Lt => i128::from(left < right),
        BinOp::Gt => i128::from(left > right),
        BinOp::Le => i128::from(left <= right),
        BinOp::Ge => i128::from(left >= right),
        BinOp::Eq => i128::from(left == right),
        BinOp::Ne => i128::from(left != right),
        BinOp::And => i128::from(left != 0 && right != 0),
        BinOp::Or => i128::from(left != 0 || right != 0),
    };
    fit(value, ty, false)
}

fn literal(value: i128, ty: Type, at: u32) -> Expr {
    let int = |value| Expr {
        ty: ty.clone(),
        kind: ExprKind::Int(value),
        at,
    };
    match value < 0 {
        true => Expr {
            ty: ty.clone(),
            kind: ExprKind::Unary(UnOp::Neg, Box::new(int(-value))),
            at,
        },
        false => int(value),
    }
}

/// Whether `value` holds, as the `int` that `&&` and `||` give.
fn truth(value: Expr, ty: Type, at: u32) -> R<Expr> {
    if let ExprKind::Binary(op, ..) = value.kind
        && (op.is_comparison() || matches!(op, BinOp::And | BinOp::Or))
    {
        return Ok(value);
    }
    if !value.ty.is_integer() {
        return Err(Unsupported::new(
            "a constant beside a test of a value that is no integer: no rule yet",
            at,
        ));
    }
    let zero = Expr {
        ty: value.ty.clone(),
        kind: ExprKind::Int(0),
        at,
    };
    Ok(Expr {
        ty,
        kind: ExprKind::Binary(BinOp::Ne, Box::new(value), Box::new(zero)),
        at,
    })
}

/// Whether evaluating `expr` changes nothing.
fn pure(expr: &Expr) -> bool {
    !expr.any(&mut |part| {
        matches!(
            part.kind,
            ExprKind::Call(..)
                | ExprKind::Assign { .. }
                | ExprKind::IncDec { .. }
                | ExprKind::Block { .. }
                | ExprKind::Assert { .. }
                | ExprKind::Init { .. }
        )
    })
}
