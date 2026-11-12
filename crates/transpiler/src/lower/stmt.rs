//! Rules for statements.

use super::{Lower, Nest, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Program, Stmt, Type, VarId, VarKind, any_expr};
use crate::facts::Slot;
use crate::rust;
use syn::parse_quote;

impl Lower<'_> {
    pub(super) fn stmts(&mut self, stmts: &[Stmt]) -> R<Vec<syn::Stmt>> {
        let mut out = Vec::new();
        for stmt in stmts {
            self.stmt(stmt, &mut out)?;
        }
        Ok(out)
    }

    /// The statements as a block of their own.
    fn block(&mut self, stmts: &[Stmt]) -> R<syn::Block> {
        Ok(rust::block(self.stmts(stmts)?))
    }

    fn stmt(&mut self, stmt: &Stmt, out: &mut Vec<syn::Stmt>) -> R<()> {
        match stmt {
            Stmt::Expr(expr) => out.extend(self.effect(expr)?),
            Stmt::Decl(var) => out.extend(self.decl(*var)?),
            Stmt::Block(body) => {
                let body = self.block(body)?;
                out.push(parse_quote!(#body));
            }
            Stmt::If(cond, then, other) => match self.decided(cond)? {
                Some(truth) => self.inline(if truth { then } else { other }, out)?,
                None => out.push(syn::Stmt::Expr(self.if_else(cond, then, other)?, None)),
            },
            Stmt::While(cond, body) => match self.decided(cond)? {
                Some(false) => {}
                Some(true) => {
                    let body = self.loop_body(body)?;
                    out.push(parse_quote!(loop #body));
                }
                None => {
                    let cond = self.cond(cond)?;
                    let body = self.loop_body(body)?;
                    out.push(parse_quote!(while #cond #body));
                }
            },
            Stmt::DoWhile(body, cond) => {
                if continues(body) {
                    return unsupported("`continue` in a `do` loop: no rule yet", cond.at);
                }
                let mut body = self.loop_body(body)?;
                let cond = rust::unary("!", self.cond(cond)?);
                body.stmts.push(parse_quote!(if #cond { break; }));
                out.push(parse_quote!(loop #body));
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => self.for_loop(init, cond.as_ref(), step.as_ref(), body, out)?,
            Stmt::Switch(value, body) => out.push(self.switch(value, body)?),
            Stmt::Break => match self.nesting.last() {
                Some(Nest::Loop) => out.push(parse_quote!(break;)),
                _ => return unsupported("`break` in the middle of a `case`: no rule yet", 0),
            },
            Stmt::Continue => out.push(parse_quote!(continue;)),
            Stmt::Return(value) => out.push(match value {
                Some(value) if self.in_main() => match self.exits {
                    true => {
                        let code = self.exit_code(value)?;
                        parse_quote!(return #code;)
                    }
                    false => parse_quote!(return;),
                },
                Some(value) => {
                    let value = self.returned(value)?;
                    parse_quote!(return #value;)
                }
                None => parse_quote!(return;),
            }),
            Stmt::IfSome {
                var,
                source,
                then,
                other,
            } => {
                let (pattern, value) = self.some(*var, source)?;
                let then = self.block(then)?;
                out.push(syn::Stmt::Expr(
                    match other.is_empty() {
                        true => parse_quote!(if let #pattern = #value #then),
                        false => {
                            let other = self.block(other)?;
                            parse_quote!(if let #pattern = #value #then else #other)
                        }
                    },
                    None,
                ));
            }
            Stmt::LetSome {
                var,
                source,
                otherwise,
            } => {
                let (pattern, value) = self.some(*var, source)?;
                let otherwise = self.block(otherwise)?;
                out.push(parse_quote!(let #pattern = #value else #otherwise;));
            }
            Stmt::Case(_) | Stmt::Default => {
                return unsupported("a `case` outside the top of its `switch`", 0);
            }
            Stmt::Goto(label) | Stmt::Label(label) => {
                return unsupported(format!("`goto {label}`: no rule yet"), 0);
            }
        }
        Ok(())
    }

    /// The statements of a branch that always runs, in the place of its
    /// `if`: in a block of their own if they declare anything.
    fn inline(&mut self, stmts: &[Stmt], out: &mut Vec<syn::Stmt>) -> R<()> {
        if stmts.iter().any(|stmt| matches!(stmt, Stmt::Decl(_))) {
            let body = self.block(stmts)?;
            out.push(parse_quote!(#body));
        } else {
            out.extend(self.stmts(stmts)?);
        }
        Ok(())
    }

    /// R10: the pattern `Some(var)` and the `Option` it opens, for a
    /// pointer that a test found not to be NULL.
    fn some(&mut self, var: VarId, source: &Expr) -> R<(syn::Pat, syn::Expr)> {
        let name = rust::ident(self.names.var(var));
        let mutability: Option<syn::Token![mut]> = self.needs_mut(var).then(Default::default);
        let repr = self.repr(Slot::Var(var), source.at)?;
        let value = self.pointer(
            source,
            super::pointer::Repr {
                nullable: true,
                ..repr
            },
        )?;
        Ok((parse_quote!(Some(#mutability #name)), value))
    }

    /// R3: `if`, with `else if` for an `else` that holds only an `if`.
    fn if_else(&mut self, cond: &Expr, then: &[Stmt], other: &[Stmt]) -> R<syn::Expr> {
        let cond = self.cond(cond)?;
        let then = self.block(then)?;
        if other.is_empty() {
            return Ok(parse_quote!(if #cond #then));
        }
        let mut other = self.stmts(other)?;
        Ok(match other.as_slice() {
            [syn::Stmt::Expr(syn::Expr::If(_), None)] => {
                let chained = other.remove(0);
                parse_quote!(if #cond #then else #chained)
            }
            _ => parse_quote!(if #cond #then else { #(#other)* }),
        })
    }

    fn loop_body(&mut self, body: &[Stmt]) -> R<syn::Block> {
        self.nesting.push(Nest::Loop);
        let body = self.block(body);
        self.nesting.pop();
        body
    }

    /// R1, R5, R6, R7, R8: a local variable. One without an initialiser
    /// starts at zero, apart from a pointer that is never NULL, which Rust
    /// lets start without a value.
    fn decl(&mut self, id: VarId) -> R<Vec<syn::Stmt>> {
        let var = &self.program.vars[id];
        if var.kind != VarKind::Local {
            // A `static` local is a field of `Globals`.
            return Ok(Vec::new());
        }
        let name = rust::ident(self.names.var(id));
        let mutability: Option<syn::Token![mut]> = self.needs_mut(id).then(Default::default);
        if var.ty.is_pointer() {
            let slot = Slot::Var(id);
            let repr = self.repr(slot, var.at)?;
            let ty = self.slot_type(slot, &var.ty, None, var.at)?;
            return Ok(vec![match &var.init {
                Some(init) => {
                    let value = self.pointer(init, repr)?;
                    parse_quote!(let #mutability #name: #ty = #value;)
                }
                None if repr.nullable => parse_quote!(let #mutability #name: #ty = None;),
                None => parse_quote!(let #mutability #name: #ty;),
            }]);
        }
        let ty = self.ty(&var.ty, var.at)?;
        let value = match &var.init {
            Some(init) => self.init(&var.ty, None, init)?,
            None => self.zero(&var.ty, var.at)?,
        };
        Ok(vec![parse_quote!(let #mutability #name: #ty = #value;)])
    }

    /// R3: a `for` loop. One that counts up to a bound that its body leaves
    /// alone is a `for` over a range; any other is a `while`.
    fn for_loop(
        &mut self,
        init: &[Stmt],
        cond: Option<&Expr>,
        step: Option<&Expr>,
        body: &[Stmt],
        out: &mut Vec<syn::Stmt>,
    ) -> R<()> {
        if let Some((var, start, bound, inclusive)) = self.counts(init, cond, step, body) {
            // A counter that the body does not read needs no name.
            let read = any_expr(
                body,
                &self.program.vars,
                &mut |part| matches!(part.kind, ExprKind::Var(named) if named == var),
            );
            let name: syn::Pat = match read {
                true => syn::Pat::Ident(syn::PatIdent {
                    attrs: Vec::new(),
                    by_ref: None,
                    mutability: None,
                    ident: rust::ident(self.names.var(var)),
                    subpat: None,
                }),
                false => parse_quote!(_),
            };
            let (start, bound) = (self.typed(&start)?, self.expr(&bound)?);
            let range: syn::Expr = match inclusive {
                true => parse_quote!(#start..=#bound),
                false => parse_quote!(#start..#bound),
            };
            let body = self.loop_body(body)?;
            out.push(parse_quote!(for #name in #range #body));
            return Ok(());
        }
        if continues(body) && step.is_some() {
            return unsupported(
                "`continue` in a `for` loop that is not a count: no rule yet",
                cond.map_or(0, |cond| cond.at),
            );
        }
        let mut stmts = self.stmts(init)?;
        let mut body = self.loop_body(body)?;
        if let Some(step) = step {
            body.stmts.extend(self.effect(step)?);
        }
        stmts.push(match cond.map(|cond| self.cond(cond)).transpose()? {
            Some(cond) => parse_quote!(while #cond #body),
            None => parse_quote!(loop #body),
        });
        // The loop's own variables end with it.
        if init.iter().any(|stmt| matches!(stmt, Stmt::Decl(_))) {
            out.push(parse_quote!({ #(#stmts)* }));
        } else {
            out.extend(stmts);
        }
        Ok(())
    }

    /// The parts of `for (int i = start; i < bound; i++)`, if the loop is
    /// one whose body changes neither `i` nor what `bound` reads.
    fn counts(
        &self,
        init: &[Stmt],
        cond: Option<&Expr>,
        step: Option<&Expr>,
        body: &[Stmt],
    ) -> Option<(VarId, Expr, Expr, bool)> {
        let program: &Program = self.program;
        let [Stmt::Decl(var)] = init else {
            return None;
        };
        let var = *var;
        let declared = &program.vars[var];
        let start = declared.init.as_ref()?;
        if declared.kind != VarKind::Local || !matches!(declared.ty, Type::Int { .. }) {
            return None;
        }
        let is_var = |expr: &Expr| matches!(expr.kind, ExprKind::Var(named) if named == var);
        let ExprKind::Binary(op @ (BinOp::Lt | BinOp::Le), left, bound) = &cond?.kind else {
            return None;
        };
        let steps = match &step?.kind {
            ExprKind::IncDec {
                target, increment, ..
            } => *increment && is_var(target),
            ExprKind::Assign {
                op: Some((BinOp::Add, _)),
                target,
                value,
            } => is_var(target) && value.constant(program) == Some(1),
            _ => false,
        };
        if !is_var(left) || !steps || bound.ty != declared.ty {
            return None;
        }
        // `i <= bound` holds again after an unsigned `i` wraps at its
        // greatest value, where a range ends; a signed `i` may not overflow.
        if *op == BinOp::Le && !matches!(declared.ty.int(), Some((32 | 64, true))) {
            return None;
        }
        // The bound is read once in Rust and before every round in C, so it
        // may hold only constants, locals that the body leaves alone and,
        // if the body stores to nothing but locals, what pointers reach.
        let mut read = vec![var];
        let mut reads_memory = false;
        let stable = !bound.any(&mut |part| match part.kind {
            ExprKind::Var(named) => {
                read.push(named);
                reads_memory |= program.vars[named].kind.is_global();
                false
            }
            ExprKind::Member { .. }
            | ExprKind::Index(..)
            | ExprKind::Deref(_)
            | ExprKind::Decay(_) => {
                reads_memory = true;
                false
            }
            ExprKind::Int(_)
            | ExprKind::Char(_)
            | ExprKind::Enumerator(_)
            | ExprKind::SizeOf(_)
            | ExprKind::Cast(_)
            | ExprKind::Binary(..)
            | ExprKind::Unary(..) => false,
            _ => true,
        });
        let is_local = |place: &Expr| matches!(place.kind, ExprKind::Var(named) if !program.vars[named].kind.is_global());
        let changed = any_expr(body, &program.vars, &mut |part| {
            let target = match &part.kind {
                ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => target,
                ExprKind::AddrOf(place) | ExprKind::Decay(place) => place,
                ExprKind::Call(..) => return reads_memory,
                _ => return false,
            };
            (reads_memory && !is_local(target))
                || target.root_var().is_some_and(|root| read.contains(&root))
        });
        (stable && !changed).then(|| (var, start.clone(), (**bound).clone(), *op == BinOp::Le))
    }

    /// R3: a `switch` whose cases each end with `break` or `return`, as a
    /// `match`.
    fn switch(&mut self, value: &Expr, body: &[Stmt]) -> R<syn::Stmt> {
        let at = value.at;
        // Each arm: its values, whether `default` is among them, and its
        // statements.
        let mut arms: Vec<(Vec<i128>, bool, &[Stmt])> = Vec::new();
        let mut rest = body;
        if !matches!(rest.first(), Some(Stmt::Case(_) | Stmt::Default) | None) {
            return unsupported("statements before the first `case`", at);
        }
        while !rest.is_empty() {
            let (mut values, mut default) = (Vec::new(), false);
            while let Some((label @ (Stmt::Case(_) | Stmt::Default), after)) = rest.split_first() {
                match label {
                    Stmt::Case(value) => values.push(*value),
                    _ => default = true,
                }
                rest = after;
            }
            let end = rest
                .iter()
                .position(|stmt| matches!(stmt, Stmt::Case(_) | Stmt::Default))
                .unwrap_or(rest.len());
            arms.push((values, default, &rest[..end]));
            rest = &rest[end..];
        }
        let value = self.expr(value)?;
        let mut cases: Vec<syn::Arm> = Vec::new();
        let mut otherwise: syn::Arm = parse_quote!(_ => {});
        let last = arms.len().saturating_sub(1);
        for (index, (values, default, stmts)) in arms.into_iter().enumerate() {
            // `case 1: { ...; break; }` is a case like any other.
            let stmts = match stmts {
                [Stmt::Block(inner)] => inner,
                _ => stmts,
            };
            let stmts = match stmts.split_last() {
                Some((Stmt::Break, before)) => before,
                Some((Stmt::Return(_) | Stmt::Continue, _)) => stmts,
                _ if index == last => stmts,
                _ => return unsupported("a `case` that falls through: no rule yet", at),
            };
            self.nesting.push(Nest::Switch);
            let body = self.block(stmts);
            self.nesting.pop();
            let body = body?;
            if default {
                otherwise = parse_quote!(_ => #body);
            } else {
                let values = values.into_iter().map(|value| rust::int(value, ""));
                cases.push(parse_quote!(#(#values)|* => #body));
            }
        }
        Ok(syn::Stmt::Expr(
            parse_quote!(match #value { #(#cases)* #otherwise }),
            None,
        ))
    }
}

/// Whether the loop body has a `continue` for this loop.
fn continues(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::Continue => true,
        Stmt::Block(body) | Stmt::Switch(_, body) => continues(body),
        Stmt::If(_, then, other) | Stmt::IfSome { then, other, .. } => {
            continues(then) || continues(other)
        }
        Stmt::LetSome { otherwise, .. } => continues(otherwise),
        _ => false,
    })
}
