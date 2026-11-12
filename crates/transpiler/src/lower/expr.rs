//! Rules for expressions. `expr` gives the value of a C expression in the
//! Rust type of its C type; `cond` gives it as a `bool`; `place` gives what
//! it names, to read or to store to.

use super::{Lower, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Type, UnOp, VarId};
use crate::facts::{Access, Slot, place_slot};
use crate::names::plain;
use crate::rust;
use syn::parse_quote;

fn symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

/// The method that wraps around as C's unsigned arithmetic does.
fn wrapping(op: BinOp) -> Option<&'static str> {
    match op {
        BinOp::Add => Some("wrapping_add"),
        BinOp::Sub => Some("wrapping_sub"),
        BinOp::Mul => Some("wrapping_mul"),
        _ => None,
    }
}

impl Lower<'_> {
    /// The value of an expression that is not a pointer.
    pub(super) fn expr(&mut self, expr: &Expr) -> R<syn::Expr> {
        self.value(expr, false)
    }

    /// The value where Rust knows the type, as in an assignment or an
    /// argument, so that a literal needs no suffix.
    pub(super) fn typed(&mut self, expr: &Expr) -> R<syn::Expr> {
        self.value(expr, true)
    }

    fn value(&mut self, expr: &Expr, typed: bool) -> R<syn::Expr> {
        let program = self.program;
        let at = expr.at;
        if let Some(literal) = self.literal(expr, typed) {
            return Ok(literal);
        }
        if expr.ty.is_pointer() {
            return unsupported("a pointer where no rule expects one", at);
        }
        match &expr.kind {
            ExprKind::Float(value) => Ok(rust::float(*value, float_suffix(&expr.ty, typed))),
            ExprKind::Enumerator(index) => {
                self.enumerators.insert(*index);
                Ok(rust::name(&program.enumerators[*index].name))
            }
            ExprKind::Zero => self.zero(&expr.ty, at),
            ExprKind::Var(_)
            | ExprKind::Member { .. }
            | ExprKind::Index(..)
            | ExprKind::Deref(_) => self.place(expr, false),
            ExprKind::Unary(op, operand) => self.unary(expr, *op, operand),
            ExprKind::Binary(op, left, right) => self.binary(expr, *op, left, right),
            ExprKind::Cast(inner) => self.cast(expr, inner, typed),
            ExprKind::Cond(cond, then, other) => match self.decided(cond)? {
                Some(true) => self.value(then, typed),
                Some(false) => self.value(other, typed),
                None => {
                    let cond = self.cond(cond)?;
                    let (then, other) = (self.value(then, typed)?, self.value(other, typed)?);
                    Ok(parse_quote!(if #cond { #then } else { #other }))
                }
            },
            ExprKind::Call(callee, args) => self.call(expr, callee, args),
            ExprKind::InitList(_) => self.init(&expr.ty, None, expr),
            // C's assignments have a value; Rust's do not.
            ExprKind::Assign { op, target, value } => {
                let assign = self.assign(expr, op.as_ref(), target, value)?;
                let place = self.place(target, false)?;
                Ok(parse_quote!({ #assign; #place }))
            }
            ExprKind::IncDec {
                target,
                increment,
                prefix,
            } => {
                let step = self.step(target, *increment)?;
                let place = self.place(target, false)?;
                Ok(match prefix {
                    true => parse_quote!({ #step; #place }),
                    false => parse_quote!({ let before = #place; #step; before }),
                })
            }
            ExprKind::Comma(first, last) => {
                let first = self.effect(first)?;
                let last = self.value(last, typed)?;
                Ok(parse_quote!({ #(#first)* #last }))
            }
            ExprKind::Str(_) => unsupported("a string literal as a value: no rule yet", at),
            ExprKind::Function(name) => {
                unsupported(format!("the function `{name}` as a value"), at)
            }
            ExprKind::Extern(name) => unsupported(format!("the library's `{name}`"), at),
            ExprKind::Int(_)
            | ExprKind::Char(_)
            | ExprKind::SizeOf(_)
            | ExprKind::Null
            | ExprKind::AddrOf(_)
            | ExprKind::Decay(_) => unsupported("an expression with no rule", at),
        }
    }

    /// R2: a constant as a literal: a number, a `sizeof`, or one of them
    /// converted. `sizeof` is the size on x86-64, where the C program ran.
    fn literal(&self, expr: &Expr, typed: bool) -> Option<syn::Expr> {
        fn folds(expr: &Expr) -> bool {
            match &expr.kind {
                ExprKind::Int(_) | ExprKind::Char(_) | ExprKind::SizeOf(_) => true,
                ExprKind::Cast(inner) => inner.ty.is_integer() && folds(inner),
                ExprKind::Unary(UnOp::Neg, inner) => matches!(inner.kind, ExprKind::Int(_)),
                // Arithmetic stays as written, unless it is about sizes.
                ExprKind::Binary(..) => {
                    expr.any(&mut |part| matches!(part.kind, ExprKind::SizeOf(_)))
                }
                _ => false,
            }
        }
        if !folds(expr) {
            return None;
        }
        if let (Type::Float { .. }, ExprKind::Cast(inner)) = (&expr.ty, &expr.kind) {
            let value = inner.constant(self.program)?;
            return Some(rust::float(value as f64, float_suffix(&expr.ty, typed)));
        }
        let value = expr.constant(self.program)?;
        Some(match expr.ty {
            Type::Bool => match value {
                0 => parse_quote!(false),
                _ => parse_quote!(true),
            },
            Type::Char => rust::byte(value as u8),
            Type::Int {
                bits: 32,
                signed: true,
            } => rust::int(value, ""),
            Type::Int { .. } if typed => rust::int(value, ""),
            Type::Int { bits, signed } => {
                rust::int(value, &format!("{}{bits}", if signed { 'i' } else { 'u' }))
            }
            _ => return None,
        })
    }

    /// R2: a conversion between numbers as `as`. A plain `char` is signed
    /// in C and a `u8` here, so it widens through `i8`.
    fn cast(&mut self, expr: &Expr, inner: &Expr, typed: bool) -> R<syn::Expr> {
        let (from, to) = (&inner.ty, &expr.ty);
        if from.is_pointer() || matches!(to, Type::Void) {
            return unsupported("a conversion with no rule", expr.at);
        }
        if *to == Type::Bool {
            return self.cond(inner);
        }
        if let (Type::Float { .. }, ExprKind::Float(value)) = (to, &inner.kind) {
            return Ok(rust::float(*value, float_suffix(to, typed)));
        }
        let value = self.expr(inner)?;
        let target = self.ty(to, expr.at)?;
        let same = |left: &Type, right: &Type| {
            let byte = Type::Int {
                bits: 8,
                signed: false,
            };
            left == right || (*left == Type::Char && *right == byte)
        };
        Ok(if same(from, to) || same(to, from) {
            value
        } else if *from == Type::Char {
            let signed = rust::cast(value, parse_quote!(i8));
            match to.int() {
                Some((8, true)) => signed,
                _ => rust::cast(signed, target),
            }
        } else {
            rust::cast(value, target)
        })
    }

    fn unary(&mut self, expr: &Expr, op: UnOp, operand: &Expr) -> R<syn::Expr> {
        Ok(match op {
            UnOp::Not => rust::call("i32::from", [self.cond(expr)?]),
            UnOp::BitNot => rust::unary("!", self.expr(operand)?),
            UnOp::Neg => match expr.ty.int() {
                Some((_, false)) => rust::method(self.expr(operand)?, "wrapping_neg", []),
                _ => rust::unary("-", self.expr(operand)?),
            },
        })
    }

    /// R2: arithmetic. Signed arithmetic is Rust's, whose overflow checks
    /// stop where C's behaviour is undefined; unsigned arithmetic wraps.
    fn binary(&mut self, expr: &Expr, op: BinOp, left: &Expr, right: &Expr) -> R<syn::Expr> {
        if op.is_comparison() || op.is_logical() {
            return Ok(rust::call("i32::from", [self.cond(expr)?]));
        }
        if left.ty.is_pointer() || right.ty.is_pointer() {
            return unsupported("pointer arithmetic as a value: no rule yet", expr.at);
        }
        let unsigned = matches!(expr.ty.int(), Some((_, false)));
        match wrapping(op) {
            Some(method) if unsigned => {
                let (left, right) = (self.expr(left)?, self.typed(right)?);
                Ok(rust::method(left, method, [right]))
            }
            _ => {
                let (left, right) = self.operands(left, right)?;
                Ok(rust::binary(left, symbol(op), right))
            }
        }
    }

    /// Two operands of one operator: a literal beside another value takes
    /// its type from that value.
    fn operands(&mut self, left: &Expr, right: &Expr) -> R<(syn::Expr, syn::Expr)> {
        let is_literal = |lower: &Self, expr: &Expr| lower.literal(expr, true).is_some();
        Ok(match (is_literal(self, left), is_literal(self, right)) {
            (true, false) => (self.typed(left)?, self.expr(right)?),
            (_, true) => (self.expr(left)?, self.typed(right)?),
            _ => (self.expr(left)?, self.expr(right)?),
        })
    }

    /// The value of a test that the facts decide. Dropping it must drop
    /// nothing that the program does.
    pub(super) fn decided(&self, cond: &Expr) -> R<Option<bool>> {
        match self.facts.truth(self.program, cond) {
            Some(_) if !cond.is_pure() => unsupported(
                "a test that the analysis decides but that calls or assigns",
                cond.at,
            ),
            truth => Ok(truth),
        }
    }

    /// R3: an expression as a `bool`, for a test.
    pub(super) fn cond(&mut self, cond: &Expr) -> R<syn::Expr> {
        if let Some(truth) = self.decided(cond)? {
            return Ok(match truth {
                true => parse_quote!(true),
                false => parse_quote!(false),
            });
        }
        match &cond.kind {
            ExprKind::Unary(UnOp::Not, inner) if inner.ty.is_pointer() => {
                Ok(rust::method(self.optional(inner)?, "is_none", []))
            }
            ExprKind::Unary(UnOp::Not, inner) => Ok(rust::unary("!", self.cond(inner)?)),
            ExprKind::Binary(op, left, right) if op.is_logical() => {
                // A side that the facts decide and that does not decide the
                // whole test adds nothing.
                if self.decided(left)?.is_some() {
                    return self.cond(right);
                }
                if self.decided(right)? == Some(*op == BinOp::And) {
                    return self.cond(left);
                }
                let (left, right) = (self.cond(left)?, self.cond(right)?);
                Ok(rust::binary(left, symbol(*op), right))
            }
            ExprKind::Binary(op, left, right) if op.is_comparison() => {
                self.comparison(*op, left, right, cond.at)
            }
            ExprKind::Cast(inner) if cond.ty == Type::Bool || inner.ty.is_pointer() => {
                self.cond(inner)
            }
            _ if cond.ty.is_pointer() => Ok(rust::method(self.optional(cond)?, "is_some", [])),
            _ if cond.ty == Type::Bool => self.expr(cond),
            _ => {
                let zero = match cond.ty {
                    Type::Float { .. } => rust::float(0.0, ""),
                    _ => rust::int(0, ""),
                };
                Ok(rust::binary(self.expr(cond)?, "!=", zero))
            }
        }
    }

    /// The `Option` that holds a pointer which may be NULL.
    fn optional(&mut self, pointer: &Expr) -> R<syn::Expr> {
        match &pointer.kind {
            ExprKind::Cast(inner) if inner.ty.is_pointer() => self.optional(inner),
            ExprKind::Var(_) | ExprKind::Member { .. } => self.place(pointer, false),
            _ => unsupported("a test of a pointer expression with no rule", pointer.at),
        }
    }

    fn comparison(&mut self, op: BinOp, left: &Expr, right: &Expr, at: u32) -> R<syn::Expr> {
        if left.ty.is_pointer() || right.ty.is_pointer() {
            let null = |expr: &Expr| matches!(expr.uncast().kind, ExprKind::Null);
            let pointer = match (null(left), null(right)) {
                (true, false) => right,
                (false, true) => left,
                _ => return unsupported("a comparison of two addresses", at),
            };
            let test = match op {
                BinOp::Eq => "is_none",
                BinOp::Ne => "is_some",
                _ => return unsupported("an ordering of a pointer and NULL", at),
            };
            return Ok(rust::method(self.optional(pointer)?, test, []));
        }
        // `c == 'a'` compares bytes: equality does not depend on the sign
        // that C's conversion to `int` gives a `char`.
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let text = |expr: &Expr| match &expr.kind {
                ExprKind::Cast(inner) if inner.ty == Type::Char => Some((**inner).clone()),
                _ => None,
            };
            let ascii = |expr: &Expr| match expr.constant(self.program) {
                Some(value @ 0..=127) => Some(value as u8),
                _ => None,
            };
            for (text, other) in [(text(left), right), (text(right), left)] {
                if let (Some(text), Some(byte)) = (text, ascii(other)) {
                    return Ok(rust::binary(
                        self.expr(&text)?,
                        symbol(op),
                        rust::byte(byte),
                    ));
                }
            }
        }
        let (left, right) = self.operands(left, right)?;
        Ok(rust::binary(left, symbol(op), right))
    }

    /// A variable by its Rust name: a local, a constant, a field of
    /// `Globals`, or `main`'s argument count.
    fn var(&mut self, var: VarId, at: u32) -> R<syn::Expr> {
        if Some(var) == self.argc {
            return Ok(parse_quote!(args.len() as i32));
        }
        if Some(var) == self.argv {
            return unsupported("`argv` other than one argument read as a number", at);
        }
        let name = self.names.var(var);
        Ok(
            match self.program.vars[var].kind.is_global() && self.facts.mutated(var) {
                true => rust::field(rust::name("globals"), name),
                false => rust::name(name),
            },
        )
    }

    /// R5, R6, R7: what an expression names. `write` says whether the place
    /// is stored to, which decides how an `Option` on the way is opened.
    pub(super) fn place(&mut self, place: &Expr, write: bool) -> R<syn::Expr> {
        let program = self.program;
        match &place.kind {
            ExprKind::Var(var) => self.var(*var, place.at),
            ExprKind::Member {
                base,
                record,
                field,
                arrow,
            } => {
                let base = match arrow {
                    true => self.pointee(base, write, true)?,
                    false => self.place(base, write)?,
                };
                let field = &program.records[*record].fields[*field];
                Ok(rust::field(base, &plain(&field.name)))
            }
            ExprKind::Index(base, index) => {
                if index.constant(program) == Some(0) && self.points_to_one(base) {
                    return self.pointee(base, write, false);
                }
                let index = self.index_value(index)?;
                Ok(rust::index(self.slice(base, write)?, index))
            }
            ExprKind::Deref(pointer) => self.pointee(pointer, write, false),
            _ => unsupported("a place with no rule", place.at),
        }
    }

    /// R1: a call of one of the program's functions or of the library.
    pub(super) fn call(&mut self, call: &Expr, callee: &Expr, args: &[Expr]) -> R<syn::Expr> {
        let program = self.program;
        let Some(name) = callee.function() else {
            return unsupported("a call through a pointer", call.at);
        };
        let Some(id) = program.function(name) else {
            return self.library(call, name, args);
        };
        let function = &program.functions[id];
        if name == "main" || function.params.len() != args.len() {
            return unsupported(format!("this call of `{name}`"), call.at);
        }
        let mut values = Vec::new();
        if self.facts.access(id) != Access::None {
            values.push(rust::name("globals"));
        }
        for (&param, arg) in function.params.iter().zip(args) {
            values.push(match program.vars[param].ty.is_pointer() {
                true => {
                    let repr = self.repr(Slot::Var(param), arg.at)?;
                    self.argument_pointer(arg, repr)?
                }
                false => self.typed(arg)?,
            });
        }
        Ok(rust::call(self.names.function(id), values))
    }

    /// R2: `target = value` or `target op= value`.
    pub(super) fn assign(
        &mut self,
        expr: &Expr,
        op: Option<&(BinOp, Type)>,
        target: &Expr,
        value: &Expr,
    ) -> R<syn::Expr> {
        if target.ty.is_pointer() {
            let (None, Some(slot)) = (op, place_slot(target)) else {
                return unsupported("a pointer that is moved or stored with no rule", expr.at);
            };
            let repr = self.repr(slot, expr.at)?;
            let value = self.pointer(value, repr)?;
            return Ok(rust::assign(self.place(target, true)?, value));
        }
        let Some((op, compute)) = op else {
            let value = self.init(&target.ty, None, value)?;
            return Ok(rust::assign(self.place(target, true)?, value));
        };
        let place = self.place(target, true)?;
        let narrower = match (target.ty.int(), compute.int()) {
            (Some((target, _)), Some((compute, _))) => target < compute,
            _ => false,
        };
        let bitwise = matches!(op, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor);
        if target.ty == *compute || (narrower && (bitwise || wrapping(*op).is_some())) {
            // The low bits of these operations do not depend on the wider
            // type that C computes in.
            let value = match target.ty == value.ty {
                true => self.typed(value)?,
                false => self.typed(&Expr {
                    ty: target.ty.clone(),
                    kind: ExprKind::Cast(Box::new(value.clone())),
                    at: value.at,
                })?,
            };
            let wraps = narrower || matches!(target.ty.int(), Some((_, false)));
            return match wrapping(*op) {
                Some(method) if wraps => {
                    self.simple(target)?;
                    Ok(rust::assign(
                        place.clone(),
                        rust::method(place, method, [value]),
                    ))
                }
                _ => Ok(rust::binary(place, &format!("{}=", symbol(*op)), value)),
            };
        }
        // Otherwise the operation runs in C's wider type and the result is
        // converted back.
        self.simple(target)?;
        let widened = Expr {
            ty: compute.clone(),
            kind: ExprKind::Cast(Box::new(target.clone())),
            at: target.at,
        };
        let computed = Expr {
            ty: compute.clone(),
            kind: ExprKind::Binary(*op, Box::new(widened), Box::new(value.clone())),
            at: expr.at,
        };
        let back = Expr {
            ty: target.ty.clone(),
            kind: ExprKind::Cast(Box::new(computed)),
            at: expr.at,
        };
        Ok(rust::assign(place, self.typed(&back)?))
    }

    /// A place that is named twice must not do anything when evaluated.
    fn simple(&self, place: &Expr) -> R<()> {
        match place.is_pure() {
            true => Ok(()),
            false => unsupported("a place that calls or assigns and is updated", place.at),
        }
    }

    /// R2: `target++` or `target--` for its effect.
    pub(super) fn step(&mut self, target: &Expr, increment: bool) -> R<syn::Expr> {
        let place = self.place(target, true)?;
        let (op, method) = match increment {
            true => ("+=", "wrapping_add"),
            false => ("-=", "wrapping_sub"),
        };
        match target.ty {
            Type::Float { .. } => Ok(rust::binary(place, op, rust::float(1.0, ""))),
            Type::Int {
                bits: 32 | 64,
                signed: true,
            } => Ok(rust::binary(place, op, rust::int(1, ""))),
            // C computes in `int` and keeps the low bits.
            Type::Char | Type::Int { .. } => {
                self.simple(target)?;
                Ok(rust::assign(
                    place.clone(),
                    rust::method(place, method, [rust::int(1, "")]),
                ))
            }
            _ => unsupported("`++` or `--` on a pointer: no rule yet", target.at),
        }
    }

    /// An expression evaluated for what it does, as statements.
    pub(super) fn effect(&mut self, expr: &Expr) -> R<Vec<syn::Stmt>> {
        let statement = |expr: syn::Expr| vec![syn::Stmt::Expr(expr, Some(Default::default()))];
        Ok(match &expr.kind {
            ExprKind::Assign { op, target, value } => {
                statement(self.assign(expr, op.as_ref(), target, value)?)
            }
            ExprKind::IncDec {
                target, increment, ..
            } => statement(self.step(target, *increment)?),
            ExprKind::Cast(inner) if expr.ty == Type::Void => self.effect(inner)?,
            ExprKind::Comma(first, last) => [self.effect(first)?, self.effect(last)?].concat(),
            ExprKind::Call(callee, args) => statement(self.call(expr, callee, args)?),
            // A value that is not used, as in `(void)argc;`.
            _ if expr.is_pure() => Vec::new(),
            _ => statement(self.expr(expr)?),
        })
    }
}

fn float_suffix(ty: &Type, typed: bool) -> &'static str {
    match ty {
        Type::Float { bits: 32 } if !typed => "f32",
        _ => "",
    }
}
