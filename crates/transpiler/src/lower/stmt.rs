//! Rules for statements.

use super::input::Read;
use super::{Helper, Lower, Nest, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Program, Stmt, Type, VarId, VarKind, any_expr};
use crate::facts::Slot;
use crate::normalise::leaves;
use crate::rust;
use syn::parse_quote;

impl Lower<'_> {
    pub(super) fn stmts(&mut self, stmts: &[Stmt]) -> R<Vec<syn::Stmt>> {
        if let Some(labelled) = self.labelled(stmts)? {
            return Ok(labelled);
        }
        let mut out = Vec::new();
        for stmt in stmts {
            let nesting = self.nesting.len();
            let lowered = self.stmt(stmt, &mut out);
            if lowered.is_err() {
                self.nesting.truncate(nesting);
            }
            self.listed(lowered)?;
        }
        Ok(out)
    }

    /// R48: the statements where the last of them that is a label is the
    /// target of jumps that all come before it, among these statements:
    /// the statements from the first jump to the label are a block with
    /// the label's name, which each `goto` leaves with `break`.
    fn labelled(&mut self, stmts: &[Stmt]) -> R<Option<Vec<syn::Stmt>>> {
        let Some(at) = stmts
            .iter()
            .rposition(|stmt| matches!(stmt, Stmt::Label(_)))
        else {
            return Ok(None);
        };
        let Stmt::Label(label) = &stmts[at] else {
            return Ok(None);
        };
        let function = self.function.map(|id| &self.program.functions[id]);
        let all = function.map_or(0, |function| jumps(&function.body, label));
        let Some(first) = stmts[..at]
            .iter()
            .position(|stmt| jumps(std::slice::from_ref(stmt), label) > 0)
        else {
            return Ok(None);
        };
        let (block, rest) = (&stmts[first..at], &stmts[at + 1..]);
        // A jump from anywhere else, such as one back from below the
        // label, has no block to leave.
        if jumps(block, label) != all || self.labels.contains(label) {
            return Ok(None);
        }
        // Rust has no `break` or `continue` of a loop directly in a
        // labelled block, and a local of the block ends with it.
        if leaves_loop(block, true) {
            return unsupported(
                format!(
                    "`break` or `continue` of a loop between `goto {label}` and its label: no rule yet"
                ),
                0,
            );
        }
        let vars = &self.program.vars;
        let declared = block.iter().any(|stmt| match stmt {
            Stmt::Decl(var) => any_expr(
                rest,
                vars,
                &mut |expr| matches!(expr.kind, ExprKind::Var(named) if named == *var),
            ),
            _ => false,
        });
        if declared {
            return unsupported(
                format!(
                    "a variable declared between `goto {label}` and its label and used after it: no rule yet"
                ),
                0,
            );
        }
        let mut out = self.stmts(&stmts[..first])?;
        self.labels.push(label.clone());
        let block = self.stmts(block);
        self.labels.pop();
        let (block, name) = (block?, block_label(label));
        out.push(parse_quote!(#name: { #(#block)* }));
        out.extend(self.stmts(rest)?);
        Ok(Some(out))
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
                None => match self.scan_test(cond)? {
                    Some((reads, found)) => {
                        let (found, missing) = if found { (then, other) } else { (other, then) };
                        out.extend(self.if_scanned(&reads, found, missing)?);
                    }
                    None => out.push(syn::Stmt::Expr(self.if_else(cond, then, other)?, None)),
                },
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
            } => {
                // R54: a counter that only its step moves, and upwards, is
                // at least where it starts.
                let counter = self.counter(init, step.as_ref(), body);
                self.counters.extend(counter);
                let lowered = self.for_loop(init, cond.as_ref(), step.as_ref(), body, out);
                if counter.is_some() {
                    self.counters.pop();
                }
                lowered?
            }
            Stmt::Switch(value, body) => out.push(self.switch(value, body)?),
            Stmt::Break => {
                let depth = self.switches();
                match self.nesting.last_mut() {
                    Some(Nest::Loop) => out.push(parse_quote!(break;)),
                    // R32: a `break` that is not the end of its case
                    // leaves the block around the `match`.
                    Some(Nest::Switch { left }) => {
                        *left = true;
                        let label = switch_label(depth);
                        out.push(parse_quote!(break #label;));
                    }
                    None => return unsupported("`break` outside a loop and a `switch`", 0),
                }
            }
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
            Stmt::Parse {
                number,
                error,
                end,
                call,
                start,
            } => out.extend(self.parse(*number, *error, *end, call, start.as_ref())?),
            Stmt::Case(_) | Stmt::Default => {
                return unsupported("a `case` outside the top of its `switch`", 0);
            }
            // R48: a jump to a label that follows leaves the block that
            // ends there.
            Stmt::Goto(label) if self.labels.contains(label) => {
                let label = block_label(label);
                out.push(parse_quote!(break #label;));
            }
            Stmt::Goto(label) | Stmt::Label(label) => {
                return unsupported(format!("`goto {label}`: no rule yet"), 0);
            }
        }
        Ok(())
    }

    /// R21: `strtol` and its like with an end pointer give the number and
    /// the index in the text where it ends.
    /// R37: with `error`, also whether the number was beyond its type.
    fn parse(
        &mut self,
        number: Option<VarId>,
        error: Option<VarId>,
        end: VarId,
        call: &Expr,
        start: Option<&(Expr, VarId)>,
    ) -> R<Vec<syn::Stmt>> {
        let at = call.at;
        let (name, text, base) = match &call.kind {
            ExprKind::Call(callee, args) => {
                (callee.function().unwrap_or_default(), &args[0], args.get(2))
            }
            _ => return unsupported("this call of the library: no rule yet", at),
        };
        if base.is_some_and(|base| base.constant(self.program) != Some(10)) {
            return unsupported(
                format!("this call of the library's `{name}`: no rule yet"),
                at,
            );
        }
        if self.program.vars[end].ty != Type::Index {
            return unsupported(
                "an end pointer that is not an offset of one text that stays in its place",
                at,
            );
        }
        let text = self.text(text, true)?;
        // A variable that nothing reads gets no name.
        let body = match self.function {
            Some(function) => self.program.functions[function].body.as_slice(),
            None => &[],
        };
        let read = |var: VarId| {
            any_expr(
                body,
                &self.program.vars,
                &mut |part| matches!(part.kind, ExprKind::Var(named) if named == var),
            )
        };
        let error = error.filter(|error| read(*error));
        let pattern = |lower: &Self, var: Option<VarId>| -> syn::Pat {
            match var.filter(|var| read(*var)) {
                Some(var) => {
                    let name = rust::ident(lower.names.var(var));
                    let mutability: Option<syn::Token![mut]> =
                        lower.needs_mut(var).then(Default::default);
                    parse_quote!(#mutability #name)
                }
                None => parse_quote!(_),
            }
        };
        let parsed: syn::Expr = match (name, error.is_some()) {
            ("strtoul" | "strtoull", false) => {
                self.helper(Helper::ParseUintPrefix);
                parse_quote!(parse_uint_prefix(#text))
            }
            ("strtoul" | "strtoull", true) => {
                self.helper(Helper::ParseUintChecked);
                parse_quote!(parse_uint_checked(#text))
            }
            ("strtod" | "strtof", checked) => {
                let ty = self.ty(&call.ty, at)?;
                match checked {
                    false => {
                        self.helper(Helper::ParseFloatPrefix);
                        parse_quote!(parse_float_prefix::<#ty>(#text))
                    }
                    true => {
                        self.helper(Helper::ParseFloatChecked);
                        parse_quote!(parse_float_checked::<#ty>(#text))
                    }
                }
            }
            (_, false) => {
                self.helper(Helper::ParseIntPrefix);
                parse_quote!(parse_int_prefix(#text))
            }
            (_, true) => {
                self.helper(Helper::ParseIntChecked);
                parse_quote!(parse_int_checked(#text))
            }
        };
        let number = pattern(self, number);
        let end = pattern(self, Some(end));
        let error = error.map(|error| pattern(self, Some(error))).into_iter();
        Ok(match start {
            None => vec![parse_quote!(let (#number, #end #(, #error)*) = #parsed;)],
            // The helper counts from where it starts to read.
            Some((start, read)) => {
                let read = rust::ident(self.names.var(*read));
                let start = self.expr(start)?;
                vec![
                    parse_quote!(let (#number, #read #(, #error)*) = #parsed;),
                    parse_quote!(let #end: usize = #start + #read;),
                ]
            }
        })
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
        // R47: a place that a search found is the index where it is one.
        if source.ty == Type::Found {
            return Ok((parse_quote!(Some(#name)), parse_quote!(#name)));
        }
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

    /// R11: an `if` on whether `scanf` found its values: `found` runs when
    /// it did and `missing` when it did not.
    fn if_scanned(
        &mut self,
        reads: &[Read],
        found: &[Stmt],
        missing: &[Stmt],
    ) -> R<Vec<syn::Stmt>> {
        let values = reads
            .iter()
            .filter(|read| matches!(read, Read::Value { .. }))
            .count();
        if found.is_empty()
            && leaves(missing)
            && let Some(stmts) = {
                let missing = self.block(missing)?;
                self.scan_or_leave(reads, missing)?
            }
        {
            return Ok(stmts);
        }
        let other = match missing.is_empty() {
            true => None,
            false => Some(self.block(missing)?),
        };
        // With several conversions, `missing` would be written once for
        // each.
        if values > 1 && other.is_some() {
            let cond = self.scan_cond(reads, true)?;
            let (found, other) = (self.block(found)?, other.iter());
            return Ok(vec![syn::Stmt::Expr(
                parse_quote!(if #cond #found #(else #other)*),
                None,
            )]);
        }
        let found = self.stmts(found)?;
        self.scan_chain(reads, found, other.as_ref())
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
                // A string literal is a `&str` whatever the text's
                // borrows are.
                Some(init) => match self.pointer(init, repr)? {
                    value @ syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(_),
                        ..
                    }) => parse_quote!(let #mutability #name = #value;),
                    value => parse_quote!(let #mutability #name: #ty = #value;),
                },
                None if repr.nullable => parse_quote!(let #mutability #name: #ty = None;),
                None => parse_quote!(let #mutability #name: #ty;),
            }]);
        }
        // R20: an array whose length the program computes is a `Vec`.
        if let (Type::Array(element, None), Some(len)) = (&var.ty, &var.len) {
            let (ty, zero) = (self.ty(element, var.at)?, self.zero(element, var.at)?);
            let len = self.index_value(len)?;
            return Ok(vec![
                parse_quote!(let #mutability #name: Vec<#ty> = vec![#zero; #len];),
            ]);
        }
        let ty = self.ty(&var.ty, var.at)?;
        let value = match &var.init {
            Some(init) => self.init(&var.ty, None, init)?,
            None => self.zero(&var.ty, var.at)?,
        };
        let called = var.init.as_ref().is_some_and(|init| {
            matches!(&init.kind, ExprKind::Call(callee, _)
                if callee.function().is_some_and(|name| self.program.function(name).is_some()))
        });
        // A counter whose type is open leaves open what is computed from it,
        // and a method needs the type.
        let open = var.init.as_ref().is_some_and(|init| {
            init.any(&mut |part| matches!(part.kind, ExprKind::Var(read) if self.untyped.contains(&read)))
        });
        Ok(vec![match !open && fixes_type(&value, called) {
            true => parse_quote!(let #mutability #name = #value;),
            false => parse_quote!(let #mutability #name: #ty = #value;),
        }])
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
            if !fixes_type(&start, false) && !fixes_type(&bound, false) {
                self.untyped.insert(var);
            }
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

    /// The variable of `for (int i = start; …; i++)` and its constant start,
    /// if the step adds to it and the body leaves it alone.
    fn counter(&self, init: &[Stmt], step: Option<&Expr>, body: &[Stmt]) -> Option<(VarId, i128)> {
        let program: &Program = self.program;
        let [Stmt::Decl(var)] = init else {
            return None;
        };
        let var = *var;
        let declared = &program.vars[var];
        let start = declared.init.as_ref()?.constant(program)?;
        if declared.kind != VarKind::Local || !matches!(declared.ty, Type::Int { .. }) {
            return None;
        }
        let is_var = |expr: &Expr| matches!(expr.kind, ExprKind::Var(named) if named == var);
        let steps = match &step?.kind {
            ExprKind::IncDec {
                target, increment, ..
            } => *increment && is_var(target),
            ExprKind::Assign {
                op: Some((BinOp::Add, _)),
                target,
                value,
            } => is_var(target) && value.constant(program).is_some_and(|by| by > 0),
            _ => false,
        };
        let moved = any_expr(body, &program.vars, &mut |part| match &part.kind {
            ExprKind::Assign { target, .. } | ExprKind::IncDec { target, .. } => is_var(target),
            ExprKind::AddrOf(inner) => is_var(inner),
            _ => false,
        });
        (steps && !moved).then_some((var, start))
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
    /// R45: cases that fall through into each other, as one arm.
    fn switch(&mut self, value: &Expr, body: &[Stmt]) -> R<syn::Stmt> {
        let at = value.at;
        // Each part: its labels, `None` for `default`, and its statements.
        let mut parts: Vec<Part> = Vec::new();
        let mut rest = body;
        if !matches!(rest.first(), Some(Stmt::Case(_) | Stmt::Default) | None) {
            return unsupported("statements before the first `case`", at);
        }
        while !rest.is_empty() {
            let mut labels = Vec::new();
            while let Some((label @ (Stmt::Case(_) | Stmt::Default), after)) = rest.split_first() {
                labels.push(match label {
                    Stmt::Case(value) => Some(*value),
                    _ => None,
                });
                rest = after;
            }
            let end = rest
                .iter()
                .position(|stmt| matches!(stmt, Stmt::Case(_) | Stmt::Default))
                .unwrap_or(rest.len());
            parts.push((labels, unblock(&rest[..end])));
            rest = &rest[end..];
        }
        // C's scope of a declaration goes on to the end of the `switch`,
        // and Rust's ends with its arm or its part.
        for (index, (_, stmts)) in parts.iter().enumerate() {
            let used = |var: VarId| {
                parts[index + 1..].iter().any(|(_, later)| {
                    any_expr(
                        later,
                        &self.program.vars,
                        &mut |part| matches!(part.kind, ExprKind::Var(named) if named == var),
                    )
                })
            };
            if declared(stmts).any(used) {
                return unsupported(
                    "a variable that a `case` declares and a later `case` uses: no rule yet",
                    at,
                );
            }
        }
        // A run of parts ends with one that leaves the `switch`, or with
        // the `switch`.
        let mut runs: Vec<&[Part]> = Vec::new();
        let mut first = 0;
        for (index, (_, stmts)) in parts.iter().enumerate() {
            let leaves = matches!(
                stmts.last(),
                Some(Stmt::Break | Stmt::Return(_) | Stmt::Continue)
            );
            if leaves || index + 1 == parts.len() {
                runs.push(&parts[first..=index]);
                first = index + 1;
            }
        }
        let value = self.expr(value)?;
        let mut cases: Vec<syn::Arm> = Vec::new();
        let mut otherwise: syn::Arm = parse_quote!(_ => {});
        // Whether a `break` in the middle of a case leaves the `switch`.
        let mut left = false;
        for run in runs {
            self.nesting.push(Nest::Switch { left: false });
            let arm = self.run(run);
            if let Some(Nest::Switch { left: true }) = self.nesting.pop() {
                left = true;
            }
            let (pattern, body) = arm?;
            let arm: syn::Arm = parse_quote!(#pattern => #body);
            match run.iter().any(|(labels, _)| labels.contains(&None)) {
                true => otherwise = arm,
                false => cases.push(arm),
            }
        }
        // Rust wants a label on a `continue` that leaves a block with a
        // label, and loops have none.
        if left && continues(body) {
            return unsupported(
                "`continue` in a `switch` with a `break` in the middle of a `case`: no rule yet",
                at,
            );
        }
        let matched: syn::Expr = parse_quote!(match #value { #(#cases)* #otherwise });
        Ok(syn::Stmt::Expr(
            match left {
                true => {
                    let label = switch_label(self.switches() + 1);
                    parse_quote!(#label: { #matched })
                }
                false => matched,
            },
            None,
        ))
    }

    /// The pattern and body of one arm of a `switch`, for parts that fall
    /// through into each other. Each part but the last runs only for the
    /// labels at or before it; once `default` is among those, for every
    /// value but the labels after it.
    fn run(&mut self, run: &[Part]) -> R<(syn::Pat, syn::Block)> {
        let labels = |parts: &[Part]| -> Vec<syn::Expr> {
            parts
                .iter()
                .flat_map(|(labels, _)| labels.iter().flatten())
                .map(|value| rust::int(*value, ""))
                .collect()
        };
        let default = run.iter().position(|(labels, _)| labels.contains(&None));
        let Some(((_, last), before)) = run.split_last() else {
            return Ok((parse_quote!(_), rust::block(Vec::new())));
        };
        let last = match last.split_last() {
            Some((Stmt::Break, stmts)) => stmts,
            _ => last,
        };
        if before.is_empty() {
            let pattern = match default {
                Some(_) => parse_quote!(_),
                None => {
                    let values = labels(run);
                    parse_quote!(#(#values)|*)
                }
            };
            return Ok((pattern, self.block(last)?));
        }
        let entry = rust::ident("entry");
        let mut out = Vec::new();
        for (index, (_, stmts)) in before.iter().enumerate() {
            let (values, reached) = match default {
                Some(default) if default <= index => (labels(&run[index + 1..]), false),
                _ => (labels(&run[..=index]), true),
            };
            let test: syn::Expr = match (values.as_slice(), reached) {
                ([value], true) => rust::binary(rust::name("entry"), "==", value.clone()),
                ([value], false) => rust::binary(rust::name("entry"), "!=", value.clone()),
                (_, true) => parse_quote!(matches!(#entry, #(#values)|*)),
                (_, false) => rust::unary("!", parse_quote!(matches!(#entry, #(#values)|*))),
            };
            let body = self.block(stmts)?;
            out.push(syn::Stmt::Expr(parse_quote!(if #test #body), None));
        }
        out.extend(self.stmts(last)?);
        let pattern = match default {
            Some(_) => parse_quote!(#entry),
            None => {
                let values = labels(run);
                parse_quote!(#entry @ (#(#values)|*))
            }
        };
        Ok((pattern, rust::block(out)))
    }

    /// The number of `switch` statements around the statement being
    /// translated, in its function.
    fn switches(&self) -> usize {
        let switch = |nest: &&Nest| matches!(nest, Nest::Switch { .. });
        self.nesting.iter().filter(switch).count()
    }
}

/// The label of the block around a `match`, for a `switch` at this depth:
/// one inside another needs a label of its own.
fn switch_label(depth: usize) -> syn::Lifetime {
    let name = match depth {
        0 | 1 => "'switch".to_owned(),
        depth => format!("'switch_{depth}"),
    };
    syn::Lifetime::new(&name, proc_macro2::Span::call_site())
}

/// The labels of a part of a `switch`, `None` for `default`, and its
/// statements.
type Part<'a> = (Vec<Option<i128>>, &'a [Stmt]);

/// `case 1: { ...; break; }` is a case like any other.
fn unblock(stmts: &[Stmt]) -> &[Stmt] {
    match stmts {
        [Stmt::Block(inner)] => inner,
        _ => stmts,
    }
}

/// The variables that the statements declare for the statements after
/// them.
fn declared(stmts: &[Stmt]) -> impl Iterator<Item = VarId> + '_ {
    stmts.iter().flat_map(|stmt| match stmt {
        Stmt::Decl(var) | Stmt::LetSome { var, .. } => vec![*var],
        Stmt::Parse {
            number,
            error,
            end,
            start,
            ..
        } => number
            .iter()
            .chain(error)
            .chain([end])
            .chain(start.as_ref().map(|(_, read)| read))
            .copied()
            .collect(),
        _ => Vec::new(),
    })
}

/// The label of the block that a `goto` to the C label leaves.
fn block_label(label: &str) -> syn::Lifetime {
    let name = format!("'{}", crate::names::label(label));
    syn::Lifetime::new(&name, proc_macro2::Span::call_site())
}

/// How many `goto` statements in the statements jump to the label.
fn jumps(stmts: &[Stmt], label: &str) -> usize {
    let count = |stmts: &[Stmt]| jumps(stmts, label);
    let of = |stmt: &Stmt| match stmt {
        Stmt::Goto(target) => usize::from(target == label),
        Stmt::Block(body)
        | Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::Switch(_, body) => count(body),
        Stmt::If(_, then, other) | Stmt::IfSome { then, other, .. } => count(then) + count(other),
        Stmt::For { init, body, .. } => count(init) + count(body),
        Stmt::LetSome { otherwise, .. } => count(otherwise),
        _ => 0,
    };
    stmts.iter().map(of).sum()
}

/// Whether the statements hold a `continue` of a loop around them or,
/// where `breaks` says that no `switch` is between, a `break` of one.
fn leaves_loop(stmts: &[Stmt], breaks: bool) -> bool {
    stmts.iter().any(|stmt| match stmt {
        Stmt::Continue => true,
        Stmt::Break => breaks,
        Stmt::Block(body) => leaves_loop(body, breaks),
        Stmt::Switch(_, body) => leaves_loop(body, false),
        Stmt::If(_, then, other) | Stmt::IfSome { then, other, .. } => {
            leaves_loop(then, breaks) || leaves_loop(other, breaks)
        }
        Stmt::LetSome { otherwise, .. } => leaves_loop(otherwise, breaks),
        _ => false,
    })
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

/// Whether the value alone gives a `let` that holds no pointer its type, so
/// that the type needs no annotation: a record literal, a cast, a literal
/// with a suffix or of one type only, a comparison, a conversion by a
/// number type's own function, a name, a field or an element of a value
/// that is fixed, a method whose result the receiver fixes, arithmetic on
/// a fixed operand, as a literal beside it takes its type, though the
/// count of a shift gives the left side none, or, where `called` says that
/// it calls a function of the program, a call.
fn fixes_type(value: &syn::Expr, called: bool) -> bool {
    let fixed = |value: &syn::Expr| fixes_type(value, false);
    match value {
        syn::Expr::Struct(_) | syn::Expr::Cast(_) => true,
        syn::Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            syn::Lit::Int(int) => !int.suffix().is_empty(),
            syn::Lit::Float(float) => !float.suffix().is_empty(),
            syn::Lit::Byte(_) | syn::Lit::Bool(_) => true,
            _ => false,
        },
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr,
            ..
        }) if matches!(**expr, syn::Expr::Lit(_)) => fixed(expr),
        syn::Expr::Unary(unary) => fixed(&unary.expr),
        syn::Expr::Call(call) => called || converts(&call.func),
        syn::Expr::Paren(inner) => fixes_type(&inner.expr, called),
        syn::Expr::Binary(binary) => match binary.op {
            syn::BinOp::Eq(_)
            | syn::BinOp::Ne(_)
            | syn::BinOp::Lt(_)
            | syn::BinOp::Le(_)
            | syn::BinOp::Gt(_)
            | syn::BinOp::Ge(_)
            | syn::BinOp::And(_)
            | syn::BinOp::Or(_) => true,
            syn::BinOp::Shl(_) | syn::BinOp::Shr(_) => fixed(&binary.left),
            _ => fixed(&binary.left) || fixed(&binary.right),
        },
        syn::Expr::Path(path) => !path.path.is_ident("None"),
        syn::Expr::Field(field) => fixed(&field.base),
        syn::Expr::Index(index) => fixed(&index.expr),
        syn::Expr::MethodCall(call) => match call.method.to_string().as_str() {
            "len" | "is_nan" | "is_some" | "is_none" | "is_empty" | "contains" => true,
            "abs" | "powf" | "sqrt" | "floor" | "ceil" | "to_bits" | "wrapping_add"
            | "wrapping_sub" | "wrapping_mul" | "wrapping_neg" => fixed(&call.receiver),
            _ => false,
        },
        _ => false,
    }
}

/// Whether `function` is a number type's own conversion, as `i32::from`
/// and `f32::from_bits` are.
fn converts(function: &syn::Expr) -> bool {
    let syn::Expr::Path(path) = function else {
        return false;
    };
    let segments: Vec<String> = path
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    match segments.as_slice() {
        [ty, function] => {
            matches!(function.as_str(), "from" | "from_bits")
                && matches!(
                    ty.as_str(),
                    "i8" | "i16"
                        | "i32"
                        | "i64"
                        | "u8"
                        | "u16"
                        | "u32"
                        | "u64"
                        | "usize"
                        | "f32"
                        | "f64"
                )
        }
        _ => false,
    }
}
