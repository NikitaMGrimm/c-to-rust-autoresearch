//! R68: a `goto` into a block from outside it. The label moves to the front
//! of the statement that holds it, and a flag of the function says that the
//! statement is entered at the label: `if (c) { A; L: B; }` becomes
//! `L: if (entered || c) { if (!entered) { A; } entered = 0; B; }`, and the
//! jump `entered = 1; goto L;`. The condition is not evaluated on entry, as
//! in C. Where the label is still inside a block from the jump, the
//! statement around moves it again, without clearing the flag, which the
//! inner statement still reads. R48 and R49 then translate the jump.

use crate::Unsupported;
use crate::c::{BinOp, Expr, ExprKind, Program, Stmt, Type, UnOp, Var, VarId, VarKind, any_expr};
use std::collections::HashSet;

pub fn enter(program: &mut Program) -> Result<(), Unsupported> {
    for function in 0..program.functions.len() {
        let mut body = std::mem::take(&mut program.functions[function].body);
        let mut flag = None;
        let mut moved = HashSet::new();
        let entered = loop {
            let whole = body.clone();
            match step(&mut body, &whole, program, &mut flag, &mut moved) {
                Ok(true) => {}
                Ok(false) => break Ok(()),
                Err(refusal) => break Err(refusal),
            }
        };
        if let Some(flag) = flag {
            for label in &moved {
                jump(&mut body, label, flag);
            }
            live(&mut body, flag, false, &program.vars);
            body.insert(0, Stmt::Decl(flag));
        }
        program.functions[function].body = body;
        entered?;
    }
    Ok(())
}

/// Moves one label to the front of the statement that holds it, where a
/// jump from outside that statement goes to it; false where there is none.
fn step(
    stmts: &mut Vec<Stmt>,
    whole: &[Stmt],
    program: &mut Program,
    flag: &mut Option<VarId>,
    moved: &mut HashSet<String>,
) -> Result<bool, Unsupported> {
    for index in 0..stmts.len() {
        let stmt = &stmts[index];
        // The block of the statement that holds a label, and the label.
        let found = inner(stmt).into_iter().find_map(|(part, body)| {
            body.iter()
                .enumerate()
                .find_map(|(at, inside)| match inside {
                    Stmt::Label(label)
                        if jumps(std::slice::from_ref(stmt), label) == 0
                            && jumps(whole, label) > 0 =>
                    {
                        Some((part, at, label.clone()))
                    }
                    _ => None,
                })
        });
        if let Some((part, at, label)) = found {
            let flag = *flag.get_or_insert_with(|| new_flag(program));
            let clears = !moved.contains(&label);
            stmts[index] = rewrite(
                stmts[index].clone(),
                part,
                at,
                &label,
                flag,
                clears,
                program,
            )?;
            stmts.insert(index, Stmt::Label(label.clone()));
            moved.insert(label);
            return Ok(true);
        }
        let stmt = &mut stmts[index];
        let bodies: Vec<&mut Vec<Stmt>> = match stmt {
            Stmt::Block(body) | Stmt::While(_, body) | Stmt::DoWhile(body, _) => vec![body],
            Stmt::If(_, then, other) => vec![then, other],
            Stmt::For { body, .. } => vec![body],
            _ => Vec::new(),
        };
        for body in bodies {
            if step(body, whole, program, flag, moved)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The blocks of a statement that a label in them makes R68's: 0 for the
/// body or the branch of an `if`, 1 for its `else`.
fn inner(stmt: &Stmt) -> Vec<(usize, &[Stmt])> {
    match stmt {
        Stmt::Block(body) | Stmt::While(_, body) | Stmt::DoWhile(body, _) => vec![(0, body)],
        Stmt::If(_, then, other) => vec![(0, then), (1, other)],
        Stmt::For { body, .. } => vec![(0, body)],
        _ => Vec::new(),
    }
}

fn rewrite(
    stmt: Stmt,
    part: usize,
    at: usize,
    label: &str,
    flag: VarId,
    clears: bool,
    program: &Program,
) -> Result<Stmt, Unsupported> {
    let cond = match &stmt {
        Stmt::If(cond, ..) | Stmt::While(cond, _) | Stmt::DoWhile(_, cond) => Some(cond.clone()),
        _ => None,
    };
    let cond = cond.as_ref();
    let into = |body: Vec<Stmt>| -> Result<Vec<Stmt>, Unsupported> {
        let mut before = body;
        let after = before.split_off(at + 1);
        before.pop();
        // Another label of the statement has moved already: one flag
        // cannot say which of the two the statement is entered at.
        if reads(&before, flag, &program.vars) || cond.is_some_and(|cond| names(cond, flag)) {
            return Err(Unsupported::new(
                format!(
                    "`goto {label}` into a block that another label of the same statement enters: no rule yet"
                ),
                0,
            ));
        }
        // C enters the scope of a declaration before the label without its
        // value; Rust has no such local.
        for stmt in &before {
            if let Stmt::Decl(var) = stmt
                && (program.vars[*var].len.is_some()
                    || any_expr(
                        &after,
                        &program.vars,
                        &mut |expr| matches!(expr.kind, ExprKind::Var(named) if named == *var),
                    ))
            {
                return Err(Unsupported::new(
                    format!(
                        "`goto {label}` into a block past a declaration that the part after the label reads: no rule yet"
                    ),
                    program.vars[*var].at,
                ));
            }
        }
        let mut out = Vec::new();
        if !before.is_empty() {
            out.push(Stmt::If(not(var(flag)), before, Vec::new()));
        }
        if clears {
            out.push(Stmt::Expr(store(flag, 0)));
        }
        out.extend(after);
        Ok(out)
    };
    let stmt = match stmt {
        Stmt::Block(body) => Stmt::Block(into(body)?),
        Stmt::DoWhile(body, cond) => Stmt::DoWhile(into(body)?, cond),
        Stmt::While(cond, body) => Stmt::While(or(var(flag), cond), into(body)?),
        Stmt::If(cond, then, other) if part == 0 => {
            Stmt::If(or(var(flag), cond), into(then)?, other)
        }
        Stmt::If(cond, then, other) => Stmt::If(and(not(var(flag)), cond), then, into(other)?),
        Stmt::For { .. } => {
            return Err(Unsupported::new(
                format!("`goto {label}` into a `for` loop, past its initialiser: no rule yet"),
                0,
            ));
        }
        stmt => stmt,
    };
    Ok(stmt)
}

/// Each `goto label` becomes `flag = 1; goto label;`.
fn jump(stmts: &mut Vec<Stmt>, label: &str, flag: VarId) {
    let mut index = 0;
    while index < stmts.len() {
        match &mut stmts[index] {
            Stmt::Goto(target) if target == label => {
                stmts.insert(index, Stmt::Expr(store(flag, 1)));
                index += 1;
            }
            Stmt::Block(body) | Stmt::While(_, body) | Stmt::DoWhile(body, _) => {
                jump(body, label, flag)
            }
            Stmt::Switch(_, body) => jump(body, label, flag),
            Stmt::If(_, then, other) => {
                jump(then, label, flag);
                jump(other, label, flag);
            }
            Stmt::For { init, body, .. } => {
                jump(init, label, flag);
                jump(body, label, flag);
            }
            _ => {}
        }
        index += 1;
    }
}

fn jumps(stmts: &[Stmt], label: &str) -> usize {
    stmts
        .iter()
        .map(|stmt| match stmt {
            Stmt::Goto(target) => usize::from(target == label),
            Stmt::Block(body)
            | Stmt::While(_, body)
            | Stmt::DoWhile(body, _)
            | Stmt::Switch(_, body) => jumps(body, label),
            Stmt::If(_, then, other) => jumps(then, label) + jumps(other, label),
            Stmt::For { init, body, .. } => jumps(init, label) + jumps(body, label),
            _ => 0,
        })
        .sum()
}

const INT: Type = Type::Int {
    bits: 32,
    signed: true,
};

/// A new local `int`, named apart from every variable of the program.
fn new_flag(program: &mut Program) -> VarId {
    let taken: HashSet<&str> = program
        .vars
        .iter()
        .map(|var| var.name.as_str())
        .chain(
            program
                .functions
                .iter()
                .map(|function| function.name.as_str()),
        )
        .collect();
    let name = (0..)
        .map(|n| match n {
            0 => "entered".to_string(),
            n => format!("entered_{n}"),
        })
        .find(|name| !taken.contains(name.as_str()))
        .unwrap_or_default();
    program.vars.push(Var {
        name,
        ty: INT,
        init: Some(int(0)),
        len: None,
        kind: VarKind::Local,
        at: 0,
    });
    program.vars.len() - 1
}

fn int(value: i128) -> Expr {
    Expr {
        ty: INT,
        kind: ExprKind::Int(value),
        at: 0,
    }
}

fn var(id: VarId) -> Expr {
    Expr {
        ty: INT,
        kind: ExprKind::Var(id),
        at: 0,
    }
}

fn store(flag: VarId, value: i128) -> Expr {
    Expr {
        ty: INT,
        kind: ExprKind::Assign {
            op: None,
            target: Box::new(var(flag)),
            value: Box::new(int(value)),
        },
        at: 0,
    }
}

fn not(expr: Expr) -> Expr {
    Expr {
        ty: INT,
        kind: ExprKind::Unary(UnOp::Not, Box::new(expr)),
        at: 0,
    }
}

fn or(left: Expr, right: Expr) -> Expr {
    binary(BinOp::Or, left, right)
}

fn and(left: Expr, right: Expr) -> Expr {
    binary(BinOp::And, left, right)
}

fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    let at = right.at;
    Expr {
        ty: INT,
        kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
        at,
    }
}

fn is_reset(stmt: &Stmt, flag: VarId) -> bool {
    matches!(stmt, Stmt::Expr(Expr { kind: ExprKind::Assign { op: None, target, value }, .. })
        if matches!(target.kind, ExprKind::Var(named) if named == flag)
            && matches!(value.kind, ExprKind::Int(0)))
}

fn reads(stmts: &[Stmt], flag: VarId, vars: &[Var]) -> bool {
    any_expr(
        stmts,
        vars,
        &mut |part| matches!(part.kind, ExprKind::Var(named) if named == flag),
    )
}

/// Drops each reset of the flag that nothing reads after, as the compiler
/// refuses such a store, going back from the end of the statements, where
/// `after` says whether the flag is read after them. Whether it is read
/// before them is the result. A loop, a `goto` and a `switch` keep it read.
fn live(stmts: &mut Vec<Stmt>, flag: VarId, mut after: bool, vars: &[Var]) -> bool {
    let read = |expr: &Expr| {
        expr.any(&mut |part| matches!(part.kind, ExprKind::Var(named) if named == flag))
    };
    for index in (0..stmts.len()).rev() {
        if is_reset(&stmts[index], flag) {
            if !after {
                stmts.remove(index);
            }
            after = false;
            continue;
        }
        if let Stmt::While(..) | Stmt::DoWhile(..) | Stmt::For { .. } | Stmt::Switch(..) =
            &stmts[index]
        {
            let whole = std::slice::from_ref(&stmts[index]);
            after = after || jumps_any(whole) || reads(whole, flag, vars);
            continue;
        }
        after = match &mut stmts[index] {
            Stmt::Expr(expr) => after || read(expr),
            Stmt::Block(body) => live(body, flag, after, vars),
            Stmt::If(cond, then, other) => {
                let then = live(then, flag, after, vars);
                let other = live(other, flag, after, vars);
                then || other || read(cond)
            }
            Stmt::Return(_) => false,
            Stmt::Label(_) | Stmt::Decl(_) => after,
            _ => true,
        };
    }
    after
}

fn jumps_any(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|stmt| match stmt {
        Stmt::Goto(_) => true,
        Stmt::Block(body)
        | Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::Switch(_, body) => jumps_any(body),
        Stmt::If(_, then, other) => jumps_any(then) || jumps_any(other),
        Stmt::For { init, body, .. } => jumps_any(init) || jumps_any(body),
        _ => false,
    })
}

fn names(expr: &Expr, flag: VarId) -> bool {
    expr.any(&mut |part| matches!(part.kind, ExprKind::Var(named) if named == flag))
}
