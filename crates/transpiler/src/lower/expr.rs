//! Rules for expressions. `expr` gives the value of a C expression in the
//! Rust type of its C type; `cond` gives it as a `bool`; `place` gives what
//! it names, to read or to store to.

use super::pointer::Repr;
use super::{Lower, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Program, Type, UnOp, VarId};
use crate::facts::{Access, Slot, place_slot};
use crate::normalise::is_search;
use crate::rust;
use syn::parse_quote;

/// The library functions that print and whose rules give no value: C
/// returns how much they wrote, or what.
const PRINTERS: [&str; 7] = [
    "printf", "fprintf", "puts", "fputs", "putchar", "fputc", "putc",
];

pub(super) fn symbol(op: BinOp) -> &'static str {
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
        let at = expr.at;
        if let Some(literal) = self.literal(expr, typed) {
            return Ok(literal);
        }
        if let Some(limit) = self.limit(expr)? {
            return Ok(limit);
        }
        if expr.ty.is_pointer() {
            return unsupported("a pointer where no rule expects one", at);
        }
        if expr.is_errno() {
            return unsupported("the number in `errno`, which is the platform's", at);
        }
        match &expr.kind {
            ExprKind::Float(value) => Ok(float(*value, &expr.ty, typed)),
            ExprKind::Enumerator(index) => {
                // An enumerator is a constant of type `i32` here, as it is
                // an `int` in C; GNU C gives one that is too large
                // another type.
                if i32::try_from(self.program.enumerators[*index].value).is_err() {
                    return unsupported("an enumerator beyond `int`: no rule yet", at);
                }
                self.enumerators.insert(*index);
                Ok(rust::name(self.names.enumerator(*index)))
            }
            ExprKind::Zero => self.zero(&expr.ty, at),
            ExprKind::Var(_)
            | ExprKind::Member { .. }
            | ExprKind::Index(..)
            | ExprKind::Deref(_) => self.place(expr, false),
            ExprKind::Unary(op, operand) => self.unary(expr, *op, operand),
            ExprKind::Binary(op, left, right) => self.binary(expr, *op, left, right),
            // The conversion of a conditional whose test is decided is
            // that of the part that is left, which may be a constant that
            // the conversion changes: `0 ? 0 : -1` as an `unsigned`.
            ExprKind::Cast(inner)
                if let ExprKind::Cond(cond, then, other) = &inner.kind
                    && let Some(truth) = self.decided(cond)? =>
            {
                let left = match truth {
                    true => then,
                    false => other,
                };
                let converted = Expr {
                    ty: expr.ty.clone(),
                    kind: ExprKind::Cast(left.clone()),
                    at,
                };
                self.value(&converted, typed)
            }
            ExprKind::Cast(inner) => self.convert(inner, &expr.ty, at, typed),
            ExprKind::Bits(inner) => self.bits(expr, inner),
            ExprKind::Cond(cond, then, other) => match self.decided(cond)? {
                Some(true) => self.value(then, typed),
                Some(false) => self.value(other, typed),
                None => {
                    let cond = self.cond(cond)?;
                    let (then, other) = (self.value(then, typed)?, self.value(other, typed)?);
                    Ok(parse_quote!(if #cond { #then } else { #other }))
                }
            },
            ExprKind::Call(callee, args) => {
                if let Some(reads) = self.scan(expr)? {
                    return self.scan_count(&reads);
                }
                // The macros that print have no value.
                if let Some(name) = callee.function()
                    && self.program.function(name).is_none()
                    && PRINTERS.contains(&name)
                {
                    return unsupported(format!("the value of `{name}`: no rule yet"), at);
                }
                self.call(expr, callee, args)
            }
            ExprKind::InitList(_) => self.init(&expr.ty, None, expr),
            // C's assignments have a value; Rust's do not.
            ExprKind::Assign { op, target, value } => {
                self.simple(target)?;
                let assign = self.assign(expr, op.as_ref(), target, value)?;
                let place = self.place(target, false)?;
                Ok(parse_quote!({ #assign; #place }))
            }
            ExprKind::IncDec {
                target,
                increment,
                prefix,
            } => {
                self.simple(target)?;
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
            ExprKind::SizeOf(ty) if ty.function().is_some() => {
                unsupported("`sizeof` of a pointer to a function", at)
            }
            ExprKind::Assert { .. } => unsupported("an `assert` where a value is needed", at),
            // R35: a statement expression as a block.
            ExprKind::Block { stmts, value } => {
                let Some(value) = value else {
                    return unsupported("a statement expression without a value", at);
                };
                if stmts.is_empty() {
                    return self.value(value, typed);
                }
                let stmts = self.stmts(stmts)?;
                let value = self.value(value, typed)?;
                Ok(parse_quote!({ #(#stmts)* #value }))
            }
            ExprKind::Init { .. } => unsupported("a first value without its declaration", at),
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
        fn folds(expr: &Expr, program: &Program) -> bool {
            match &expr.kind {
                ExprKind::Int(_) | ExprKind::Char(_) | ExprKind::SizeOf(_) => true,
                ExprKind::Cast(inner) => inner.ty.is_integer() && folds(inner, program),
                ExprKind::Unary(UnOp::Neg, inner) => matches!(inner.kind, ExprKind::Int(_)),
                // Arithmetic stays as written, unless it is about sizes, or
                // the compiler's lints refuse it: two constants that are
                // equal, as in `4 - 4`.
                ExprKind::Binary(op, left, right) => {
                    expr.any(&mut |part| matches!(part.kind, ExprKind::SizeOf(_)))
                        || matches!(
                            op,
                            BinOp::Sub | BinOp::Div | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
                        ) && left.constant(program).is_some()
                            && left.constant(program) == right.constant(program)
                }
                _ => false,
            }
        }
        if !folds(expr, self.program) {
            return None;
        }
        if let (Type::Float { .. }, ExprKind::Cast(inner)) = (&expr.ty, &expr.kind) {
            let value = inner.constant(self.program)?;
            return Some(float(value as f64, &expr.ty, typed));
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
            Type::Int { .. } | Type::Index if typed => rust::int(value, ""),
            Type::Int { bits, signed } => {
                rust::int(value, &format!("{}{bits}", if signed { 'i' } else { 'u' }))
            }
            Type::Index => rust::int(value, "usize"),
            _ => return None,
        })
    }

    /// R2: the least value of a signed type, which C writes as arithmetic,
    /// `-2147483647 - 1` for `INT_MIN`, is `i32::MIN`.
    fn limit(&self, expr: &Expr) -> R<Option<syn::Expr>> {
        let Type::Int { bits, signed: true } = expr.ty else {
            return Ok(None);
        };
        if let ExprKind::Binary(BinOp::Sub, left, right) = &expr.kind
            && let ExprKind::Unary(UnOp::Neg, greatest) = &left.kind
            && let ExprKind::Int(greatest) = greatest.uncast().kind
            && let ExprKind::Int(1) = right.uncast().kind
            && left.ty == expr.ty
            && greatest == (1 << (bits - 1)) - 1
        {
            let ty = self.ty(&expr.ty, expr.at)?;
            return Ok(Some(parse_quote!(#ty::MIN)));
        }
        Ok(None)
    }

    /// R2: a conversion between numbers as `as`. A plain `char` is signed
    /// in C and a `u8` here, so it widens through `i8`.
    pub(super) fn convert(
        &mut self,
        inner: &Expr,
        to: &Type,
        at: u32,
        typed: bool,
    ) -> R<syn::Expr> {
        let from = &inner.ty;
        if from.is_pointer() || matches!(to, Type::Void) {
            return unsupported("a conversion with no rule", at);
        }
        if *to == Type::Bool {
            return self.cond(inner);
        }
        // A literal of the other floating-point type, where Rust reads the
        // same digits as the number that C's conversion gives.
        if let (Type::Float { bits }, ExprKind::Float(value)) = (to, &inner.kind) {
            let narrowed = *value as f32;
            let same = match bits {
                _ if inner.ty == *to => true,
                32 => format!("{value:?}").parse() == Ok(narrowed),
                _ => f64::from(narrowed) == *value,
            };
            if same {
                return Ok(float(*value, to, typed));
            }
        }
        if let Some(original) = round_trip(inner, to) {
            return self.value(original, typed);
        }
        // `From` is no `const` function, so a global's value converts with
        // `as`.
        if let Some(limit) = self.limit(inner)?
            && self.function.is_some()
            && from != to
            && widens(from, to)
            && to.int().is_some_and(|(_, signed)| signed)
        {
            let target = self.ty(to, at)?;
            return Ok(parse_quote!(#target::from(#limit)));
        }
        // A conversion that only widens keeps the value, so the one after it
        // says the same alone: `(unsigned)(int)c` is `c as i8 as u32`.
        if let ExprKind::Cast(original) = &inner.kind
            && widens(&original.ty, from)
        {
            return self.convert(original, to, at, typed);
        }
        // `strlen` counts in a `usize`, which converts as its `u64` does.
        if let Some(length) = self.length(inner)? {
            return Ok(match to {
                Type::Index => length,
                _ => rust::cast(length, self.ty(to, at)?),
            });
        }
        let target = self.ty(to, at)?;
        if *from == Type::Bool && to.is_integer() {
            // R37 writes `!range` as a `bool`, which is a test.
            return Ok(rust::cast(self.cond(inner)?, target));
        }
        let value = self.expr(inner)?;
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

    /// R33: the bits of a floating-point number, or the number that has
    /// these bits.
    fn bits(&mut self, expr: &Expr, inner: &Expr) -> R<syn::Expr> {
        match (&inner.ty, &expr.ty) {
            (Type::Float { .. }, Type::Int { .. }) => {
                // A method needs a receiver whose type Rust knows, which
                // a literal and arithmetic on literals do not have.
                let named = matches!(
                    inner.uncast().kind,
                    ExprKind::Var(_)
                        | ExprKind::Member { .. }
                        | ExprKind::Index(..)
                        | ExprKind::Deref(_)
                        | ExprKind::Call(..)
                );
                Ok(match (named, self.ty(&inner.ty, expr.at)?) {
                    (true, _) => rust::method(self.expr(inner)?, "to_bits", []),
                    (false, float) => {
                        let value = self.typed(inner)?;
                        parse_quote!(#float::to_bits(#value))
                    }
                })
            }
            (Type::Int { .. }, Type::Float { .. }) => {
                let (float, bits) = (self.ty(&expr.ty, expr.at)?, self.typed(inner)?);
                Ok(parse_quote!(#float::from_bits(#bits)))
            }
            _ => unsupported("the bits of a value that is no number", expr.at),
        }
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
            // A test that the facts decide is the number itself.
            if let Some(truth) = self.decided(expr)? {
                return Ok(rust::int(i128::from(truth), ""));
            }
            return Ok(rust::call("i32::from", [self.cond(expr)?]));
        }
        if left.ty.is_pointer() || right.ty.is_pointer() {
            return unsupported("pointer arithmetic as a value: no rule yet", expr.at);
        }
        if expr.ty == Type::Index {
            return self.index_arithmetic(op, left, right);
        }
        // The compiler's lints refuse these, such as `x - x`.
        if matches!(
            op,
            BinOp::Sub | BinOp::Div | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
        ) && same_value(left, right)
        {
            return unsupported(
                format!(
                    "the same value on both sides of `{}`: no rule yet",
                    symbol(op)
                ),
                expr.at,
            );
        }
        let right = shift_count(op, right);
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

    /// Arithmetic on an index, which is Rust's: an index that leaves its
    /// array is undefined in C and stops the program here. An offset that
    /// is signed and not a constant may be negative, so it is added as the
    /// number it is; where C's result is inside the array, this one is the
    /// same.
    fn index_arithmetic(&mut self, op: BinOp, left: &Expr, right: &Expr) -> R<syn::Expr> {
        if let Some(offset) = signed_offset(right) {
            let offset = self.expr(offset)?;
            let left = self.expr(left)?;
            return Ok(match op {
                BinOp::Add => {
                    let offset = rust::cast(offset, parse_quote!(isize));
                    rust::method(left, "wrapping_add_signed", [offset])
                }
                BinOp::Sub => {
                    let offset = rust::cast(offset, parse_quote!(usize));
                    rust::method(left, "wrapping_sub", [offset])
                }
                _ => return unsupported("arithmetic on an index with no rule", right.at),
            });
        }
        let (left, right) = self.operands(left, right)?;
        Ok(rust::binary(left, symbol(op), right))
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
            // R59: `realloc` is never NULL to the facts, as `malloc` is,
            // but its result is not stored back where the test is.
            Some(_) if calls_realloc(cond) => {
                unsupported(super::pointer::REALLOC_ELSEWHERE, cond.at)
            }
            Some(_) if !cond.is_pure() => unsupported(
                "a test that the analysis decides but that calls or assigns",
                cond.at,
            ),
            truth => Ok(truth),
        }
    }

    /// R3: an expression as a `bool`, for a test.
    pub(super) fn cond(&mut self, cond: &Expr) -> R<syn::Expr> {
        // R37 rewrote each read of `errno` that is one call's error.
        let errno = match &cond.kind {
            ExprKind::Unary(UnOp::Not, inner) => inner.is_errno(),
            ExprKind::Binary(BinOp::Eq | BinOp::Ne, left, right) => {
                left.is_errno() || right.is_errno()
            }
            _ => cond.is_errno(),
        };
        if errno {
            return unsupported("a read of `errno` that is not one call's error", cond.at);
        }
        if calls_realloc(cond) {
            return unsupported(super::pointer::REALLOC_ELSEWHERE, cond.at);
        }
        if let Some(truth) = self.decided(cond)? {
            return Ok(match truth {
                true => parse_quote!(true),
                false => parse_quote!(false),
            });
        }
        if let Some((reads, found)) = self.scan_test(cond)? {
            return self.scan_cond(&reads, found);
        }
        // `!!x` tests what `x` tests.
        if let ExprKind::Unary(UnOp::Not, inner) = &cond.kind
            && let ExprKind::Unary(UnOp::Not, twice) = &inner.kind
        {
            return self.cond(twice);
        }
        // R30: `strcmp(a, b)` as a test, and `!strcmp(a, b)`.
        let sign = match &cond.kind {
            ExprKind::Unary(UnOp::Not, inner) => self.sign_test(inner, BinOp::Eq),
            _ => self.sign_test(cond, BinOp::Ne),
        };
        if let Some(test) = sign {
            return test;
        }
        match &cond.kind {
            ExprKind::Unary(UnOp::Not, inner)
                if inner.ty.is_pointer() || inner.ty == Type::Found =>
            {
                self.is_set(inner, false)
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
                // The same test twice is one test, and the compiler's
                // lints refuse it twice. What `same_value` knows has no
                // effect.
                if same_value(left, right) {
                    return self.cond(left);
                }
                let (left, right) = (self.cond(left)?, self.cond(right)?);
                Ok(rust::binary(left, symbol(*op), right))
            }
            ExprKind::Binary(op, left, right) if op.is_comparison() => {
                self.comparison(*op, left, right, cond.at)
            }
            ExprKind::Cast(inner)
                if cond.ty == Type::Bool
                    || inner.ty == Type::Bool
                    || inner.ty.is_pointer()
                    || inner.ty == Type::Found =>
            {
                self.cond(inner)
            }
            _ if cond.ty.is_pointer() || cond.ty == Type::Found => self.is_set(cond, true),
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

    /// Whether a pointer is not NULL, or whether it is.
    fn is_set(&mut self, pointer: &Expr, set: bool) -> R<syn::Expr> {
        // `fgets` returns its buffer, or NULL where it read nothing.
        if let ExprKind::Call(callee, args) = &pointer.uncast().kind
            && let (Some("fgets"), [buffer, size, stream]) = (callee.function(), args.as_slice())
        {
            let read = self.read_line(buffer, size, stream, pointer.at)?;
            return Ok(match set {
                true => read,
                false => rust::unary("!", read),
            });
        }
        // R39: `getenv` that is only tested.
        if let ExprKind::Call(callee, args) = &pointer.uncast().kind
            && super::pointer::is_getenv(self.program, callee)
        {
            let test = if set { "is_ok" } else { "is_err" };
            return Ok(rust::method(self.environment(args, pointer.at)?, test, []));
        }
        // R40: a search that is only tested.
        if is_search(self.program, pointer) {
            let found = self.search_found(pointer.uncast())?;
            return Ok(match set {
                true => found,
                false => rust::unary("!", found),
            });
        }
        let test = if set { "is_some" } else { "is_none" };
        Ok(rust::method(self.optional(pointer)?, test, []))
    }

    /// The `Option` that holds a pointer which may be NULL.
    fn optional(&mut self, pointer: &Expr) -> R<syn::Expr> {
        match &pointer.kind {
            ExprKind::Cast(inner) if inner.ty.is_pointer() || inner.ty == Type::Found => {
                self.optional(inner)
            }
            ExprKind::Var(_) | ExprKind::Member { .. } => {
                // The analysis leaves the test only of a pointer that may
                // be NULL, and of one that may be `argv[argc]`.
                if let Some(slot) = place_slot(pointer)
                    && !self.repr(slot, pointer.at)?.nullable
                {
                    return unsupported(
                        "a test for NULL of a pointer to a program argument: no rule yet",
                        pointer.at,
                    );
                }
                self.place(pointer, false)
            }
            _ => unsupported("a test of a pointer expression with no rule", pointer.at),
        }
    }

    fn comparison(&mut self, op: BinOp, left: &Expr, right: &Expr, at: u32) -> R<syn::Expr> {
        if left.ty.is_pointer() || right.ty.is_pointer() {
            let null = |expr: &Expr| matches!(expr.uncast().kind, ExprKind::Null);
            let pointer = match (null(left), null(right)) {
                (true, false) => right,
                (false, true) => left,
                _ if left.ty.function().is_some() || right.ty.function().is_some() => {
                    // Rust gives two functions with one body one address,
                    // or one function two.
                    return unsupported("a comparison of two functions", at);
                }
                _ => return self.same_address(op, left, right, at),
            };
            return match op {
                BinOp::Eq => self.is_set(pointer, false),
                BinOp::Ne => self.is_set(pointer, true),
                _ => unsupported("an ordering of a pointer and NULL", at),
            };
        }
        if same_value(left, right) {
            // `x != x` holds only for a NaN, and Rust has a name for the
            // test.
            if let (BinOp::Eq | BinOp::Ne, Type::Float { .. }) = (op, &left.ty) {
                let nan = rust::method(self.expr(left)?, "is_nan", []);
                return Ok(match op {
                    BinOp::Ne => nan,
                    _ => rust::unary("!", nan),
                });
            }
            // The compiler's lints refuse such a comparison.
            return unsupported("a comparison of a value with itself: no rule yet", at);
        }
        // `n >= INT_MAX` holds for one value, and the compiler's lints
        // refuse to have it said otherwise.
        for (value, constant, op) in [(left, right, op), (right, left, op.mirrored())] {
            if value.constant(self.program).is_none()
                && let Some(constant) = constant.constant(self.program)
                && let Some(range) = value.range()
                && let Some(Err(equal)) = Expr::limited(op, range, constant)
            {
                return self.comparison(equal, left, right, at);
            }
        }
        // R30: a comparing function of `string.h` against 0, whose sign is
        // all that C fixes.
        for (call, zero, op) in [(left, right, op), (right, left, op.mirrored())] {
            if zero.constant(self.program) == Some(0)
                && let Some(test) = self.sign_test(call, op)
            {
                return test;
            }
        }
        // `*text == '\0'`: the text is empty.
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            for (text, other) in [(left, right), (right, left)] {
                if let ExprKind::Cast(inner) = &text.kind
                    && let ExprKind::Deref(pointer) = &inner.kind
                    && self.is_text(pointer)
                    && other.constant(self.program) == Some(0)
                {
                    // R21: text ends at the index that is its length.
                    if let ExprKind::Binary(BinOp::Add, text, index) = &pointer.kind {
                        let (text, index) = self.text_at(text, index)?;
                        let len = rust::method(text, "len", []);
                        return Ok(rust::binary(index, symbol(op), len));
                    }
                    let empty = rust::method(self.pointer(pointer, Repr::TEXT)?, "is_empty", []);
                    return Ok(match op {
                        BinOp::Eq => empty,
                        _ => rust::unary("!", empty),
                    });
                }
            }
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

    /// R21: whether two pointers into text point to the same place, as
    /// `end == text` asks after `strtol`: Rust compares where the two
    /// `&str` start.
    fn same_address(&mut self, op: BinOp, left: &Expr, right: &Expr, at: u32) -> R<syn::Expr> {
        let is_text = |lower: &Self, side: &Expr| lower.is_text(side) || lower.is_argument(side);
        if !matches!(op, BinOp::Eq | BinOp::Ne) || !is_text(self, left) || !is_text(self, right) {
            return unsupported("a comparison of two addresses", at);
        }
        // Two arguments that are empty are two places in C and one in
        // Rust, where an empty `String` has no memory of its own.
        let holds_argument = |side: &Expr| {
            // The end of a number in text is in the memory of the text.
            let side = match &side.kind {
                ExprKind::Binary(BinOp::Add, text, index) if index.ty == Type::Index => text,
                _ => side,
            };
            self.is_argument(side)
                || place_slot(side.uncast()).is_some_and(|slot| self.facts.pointer(slot).argument)
        };
        if holds_argument(left) && holds_argument(right) {
            return unsupported("a comparison of the addresses of two program arguments", at);
        }
        // NULL has no address to compare in Rust.
        for side in [left, right] {
            if let Some(slot) = place_slot(side.uncast())
                && self.repr(slot, at)?.nullable
            {
                return unsupported(
                    "a comparison of two addresses of which one may be NULL: no rule yet",
                    at,
                );
            }
        }
        let start = |lower: &mut Self, side: &Expr| {
            let text = match lower.is_argument(side) {
                true => lower.argument(side)?,
                false => lower.pointer(side, Repr::TEXT)?,
            };
            Ok(rust::method(text, "as_ptr", []))
        };
        let (left, right) = (start(self, left)?, start(self, right)?);
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
                Ok(rust::field(base, self.names.field(*record, *field)))
            }
            ExprKind::Index(base, index) => {
                if index.constant(program) == Some(0) && self.points_to_one(base) {
                    return self.pointee(base, write, false);
                }
                let index = self.index_value(index)?;
                Ok(rust::index(self.slice(base, write)?, index))
            }
            ExprKind::Deref(pointer) => self.pointee(pointer, write, false),
            // R60: a compound literal that is only read, for one of its fields.
            ExprKind::InitList(_) if !write => self.expr(place),
            _ => unsupported("a place with no rule", place.at),
        }
    }

    /// R1: a call of one of the program's functions or of the library.
    pub(super) fn call(&mut self, call: &Expr, callee: &Expr, args: &[Expr]) -> R<syn::Expr> {
        let program = self.program;
        let Some(name) = callee.function() else {
            return self.call_through(call, callee, args);
        };
        let Some(id) = program.function(name) else {
            return self.library(call, name, args);
        };
        let function = &program.functions[id];
        if name == "main" || function.params.len() != args.len() {
            return unsupported(format!("this call of `{name}`"), call.at);
        }
        // Rust lends memory to a function once if it may store there, and
        // gives it away once.
        let mut lent: Vec<(usize, Vec<Part>, bool)> = Vec::new();
        // R54: the two arguments that are elements of one array, the first
        // of them before the second.
        let mut apart: Option<(usize, usize)> = None;
        for (position, (&param, arg)) in function.params.iter().zip(args).enumerate() {
            let ty = &program.vars[param].ty;
            if !ty.is_pointer() || ty.function().is_some() {
                continue;
            }
            let Some(memory) = pointed(arg) else {
                continue;
            };
            let repr = self.repr(Slot::Var(param), arg.at)?;
            let stores = repr.mutable || repr.owned;
            let shared = |other: &[Part]| {
                let common = other.len().min(memory.len());
                other[..common] == memory[..common]
            };
            let overlaps: Vec<usize> = lent
                .iter()
                .filter(|(_, other, other_stores)| (stores || *other_stores) && shared(other))
                .map(|(other, ..)| *other)
                .collect();
            match (overlaps.as_slice(), apart) {
                ([], _) => {}
                ([other], None) if self.before(&args[*other], arg).is_some() => {
                    apart = match self.before(&args[*other], arg) {
                        Some(true) => Some((*other, position)),
                        _ => Some((position, *other)),
                    };
                }
                _ => {
                    return unsupported(
                        "two arguments that point into the same memory, one of them to store through",
                        arg.at,
                    );
                }
            }
            lent.push((position, memory, stores));
        }
        // A global that changes is a field of `Globals`, which the callee
        // borrows as a whole: beside `&Globals` only a shared borrow of a
        // field fits, and beside `&mut Globals` none.
        let access = self.facts.access(id);
        for (_, memory, stores) in &lent {
            let global = match memory.first() {
                Some(Part::Var(var)) if !memory.contains(&Part::Target) => {
                    program.vars[*var].kind.is_global() && self.facts.mutated(*var)
                }
                _ => false,
            };
            if global && (access == Access::Write || access == Access::Read && *stores) {
                return unsupported(
                    "a pointer to a global handed to a function that uses the globals",
                    call.at,
                );
            }
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
        self.split = apart.is_some();
        let Some((low, high)) = apart else {
            return Ok(rust::call(self.names.function(id), values));
        };
        // The array is split before the second element, so that each
        // argument borrows a part of its own.
        let offset = values.len() - args.len();
        let element = |value: &syn::Expr| match value {
            syn::Expr::Reference(syn::ExprReference {
                mutability, expr, ..
            }) => match &**expr {
                // An element, not the rest of the array from it, which
                // would reach the other argument.
                syn::Expr::Index(syn::ExprIndex { expr, index, .. })
                    if !matches!(**index, syn::Expr::Range(_)) =>
                {
                    Some((*mutability, (**expr).clone(), (**index).clone()))
                }
                _ => None,
            },
            _ => None,
        };
        let (Some((low_mut, array, low_index)), Some((high_mut, other, high_index))) = (
            element(&values[offset + low]),
            element(&values[offset + high]),
        ) else {
            return unsupported(
                "two arguments that point into the same memory, one of them to store through",
                call.at,
            );
        };
        let tokens = |side: &syn::Expr| quote::ToTokens::to_token_stream(side).to_string();
        if tokens(&array) != tokens(&other) {
            return unsupported(
                "two arguments that point into the same memory, one of them to store through",
                call.at,
            );
        }
        values[offset + low] = parse_quote!(&#low_mut lower[#low_index]);
        values[offset + high] = parse_quote!(&#high_mut upper[0]);
        let call = rust::call(self.names.function(id), values);
        Ok(parse_quote!({
            let (lower, upper) = #array.split_at_mut(#high_index);
            #call
        }))
    }

    /// R54: whether the first of two addresses of elements of one array,
    /// `&a[i]` and `&a[j]`, is before the second on every run, or after it:
    /// both indices are constants, or one is a constant and the other the
    /// counter of a `for` loop that starts above it.
    fn before(&self, first: &Expr, second: &Expr) -> Option<bool> {
        let element = |pointer: &'_ Expr| match &pointer.uncast().kind {
            ExprKind::AddrOf(place) => match &place.kind {
                ExprKind::Index(array, index) => Some(((**array).clone(), (**index).clone())),
                _ => None,
            },
            _ => None,
        };
        let ((array, first), (other, second)) = (element(first)?, element(second)?);
        if !same_place(&array, &other) || !array.is_pure() {
            return None;
        }
        // One index is the other and a constant more, `&a[i]` and
        // `&a[i + 1]`: the sum of a signed `int` or wider does not wrap.
        let more = |index: &Expr, base: &Expr| {
            let ExprKind::Binary(op @ (BinOp::Add | BinOp::Sub), left, right) =
                &counted(index).kind
            else {
                return None;
            };
            let base = counted(base);
            let signed = matches!(base.ty.int(), Some((32 | 64, true))) && base.ty == left.ty;
            let (base_side, constant) = match op {
                BinOp::Add if same_value(right, base) => (right, left),
                _ => (left, right),
            };
            match constant.constant(self.program) {
                Some(by) if signed && same_value(base_side, base) && base.is_pure() => {
                    Some(if *op == BinOp::Add { by } else { -by })
                }
                _ => None,
            }
        };
        match (more(&second, &first), more(&first, &second)) {
            (Some(by), _) if by != 0 => return Some(by > 0),
            (_, Some(by)) if by != 0 => return Some(by < 0),
            _ => {}
        }
        // The least value an index takes.
        let least = |index: &Expr| match counted(index).kind {
            ExprKind::Var(var) => self
                .counters
                .iter()
                .find(|(counter, _)| *counter == var)
                .map(|(_, start)| (*start, false)),
            _ => index.constant(self.program).map(|value| (value, true)),
        };
        match (least(&first)?, least(&second)?) {
            ((first, true), (second, _)) if (0..second).contains(&first) => Some(true),
            ((first, _), (second, true)) if (0..first).contains(&second) => Some(false),
            _ => None,
        }
    }

    /// R2: `target = value` or `target op= value`.
    pub(super) fn assign(
        &mut self,
        expr: &Expr,
        op: Option<&(BinOp, Type)>,
        target: &Expr,
        value: &Expr,
    ) -> R<syn::Expr> {
        if target.is_errno() {
            return unsupported("a store to `errno` other than 0 before a call", expr.at);
        }
        if target.ty.function().is_some() && op.is_none() {
            let value = match place_slot(target) {
                Some(slot) => {
                    let nullable = self.repr(slot, expr.at)?.nullable;
                    self.function_value(value, nullable)?
                }
                None => self.element_value(value)?,
            };
            return Ok(rust::assign(self.place(target, true)?, value));
        }
        if target.ty.is_pointer() {
            let (None, Some(slot)) = (op, place_slot(target)) else {
                return unsupported("a pointer that is moved or stored with no rule", expr.at);
            };
            if let ExprKind::Call(callee, args) = &value.uncast().kind
                && callee.function() == Some("realloc")
                && let [grown, bytes] = args.as_slice()
                && same_value(grown.uncast(), target)
            {
                return self.resize(target, bytes);
            }
            let repr = self.repr(slot, expr.at)?;
            // `p = p->next`: the one read of `p` is the pointer that the
            // field is reached through.
            let through = |value: &Expr| match &value.kind {
                ExprKind::Member {
                    base, arrow: true, ..
                } => match base.uncast().kind {
                    ExprKind::Var(var) => Some(var),
                    _ => None,
                },
                _ => None,
            };
            self.walking = match target.kind {
                ExprKind::Var(var) if through(value) == Some(var) => Some(var),
                _ => None,
            };
            // Any other value that reads `p` borrows it while it is
            // assigned.
            if let ExprKind::Var(var) = target.kind
                && self.walking.is_none()
                && repr.nullable
                && repr.mutable
                && !repr.owned
                && value.any(&mut |read| matches!(read.kind, ExprKind::Var(read) if read == var))
            {
                return unsupported(
                    "a pointer that is stored through and assigned from itself other than by one field: no rule yet",
                    expr.at,
                );
            }
            let value = self.pointer(value, repr);
            self.walking = None;
            return Ok(rust::assign(self.place(target, true)?, value?));
        }
        let width = self.bit_field(target);
        let Some((op, compute)) = op else {
            // `x = x op y` is `x op= y` where the place does nothing when
            // evaluated and no conversion sits between the operation and
            // the store; a shift may take another type on its right. Rust
            // evaluates the right side of `op=` first, so it must not call
            // or assign either.
            if let ExprKind::Binary(op, left, right) = &value.kind
                && width.is_none()
                && target.ty != Type::Index
                && value.ty == target.ty
                && right.ty == target.ty
                && matches!(
                    op,
                    BinOp::Add
                        | BinOp::Sub
                        | BinOp::Mul
                        | BinOp::Div
                        | BinOp::Rem
                        | BinOp::BitAnd
                        | BinOp::BitOr
                        | BinOp::BitXor
                )
                && target.is_pure()
                && right.is_pure()
                && same_value(left, target)
                && !same_value(left, right)
            {
                return self.assign(expr, Some(&(*op, target.ty.clone())), target, right);
            }
            let value = self.init(&target.ty, None, value)?;
            let value = self.in_bits(value, &target.ty, width);
            return Ok(rust::assign(self.place(target, true)?, value));
        };
        if width.is_some() {
            return unsupported("an update of a bit-field in place: no rule yet", expr.at);
        }
        let place = self.place(target, true)?;
        let narrower = match (target.ty.int(), compute.int()) {
            (Some((target, _)), Some((compute, _))) => target < compute,
            _ => false,
        };
        let bitwise = matches!(op, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor);
        if target.ty == Type::Index {
            if signed_offset(value).is_some() {
                self.simple(target)?;
                return Ok(rust::assign(
                    place,
                    self.index_arithmetic(*op, target, value)?,
                ));
            }
            let value = self.typed(value)?;
            return Ok(rust::binary(place, &format!("{}=", symbol(*op)), value));
        }
        if target.ty == *compute || (narrower && (bitwise || wrapping(*op).is_some())) {
            // The low bits of these operations do not depend on the wider
            // type that C computes in.
            let count = shift_count(*op, value);
            let value = match target.ty == value.ty || !std::ptr::eq(count, value) {
                true => self.typed(count)?,
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

    /// A place that is named twice must not do anything when evaluated: Rust
    /// would do it twice.
    fn simple(&self, place: &Expr) -> R<()> {
        match place.is_pure() {
            true => Ok(()),
            false => unsupported("a place that calls or assigns and is updated", place.at),
        }
    }

    /// R2: `target++` or `target--` for its effect.
    pub(super) fn step(&mut self, target: &Expr, increment: bool) -> R<syn::Expr> {
        if self.bit_field(target).is_some() {
            return unsupported("an update of a bit-field in place: no rule yet", target.at);
        }
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
            }
            | Type::Index => Ok(rust::binary(place, op, rust::int(1, ""))),
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
        if let Some(reads) = self.scan(expr)? {
            return self.scan_chain(&reads, Vec::new(), None);
        }
        Ok(match &expr.kind {
            ExprKind::Assign { op, target, value } => {
                statement(self.assign(expr, op.as_ref(), target, value)?)
            }
            ExprKind::IncDec {
                target, increment, ..
            } => statement(self.step(target, *increment)?),
            ExprKind::Cast(inner) if expr.ty == Type::Void => self.effect(inner)?,
            ExprKind::Comma(first, last) => [self.effect(first)?, self.effect(last)?].concat(),
            ExprKind::Call(callee, args) => {
                let call = self.call(expr, callee, args)?;
                match (std::mem::take(&mut self.split), call) {
                    // R54: as a statement, the call follows the split.
                    (true, syn::Expr::Block(block)) => {
                        let mut stmts = block.block.stmts;
                        if let Some(syn::Stmt::Expr(_, semi)) = stmts.last_mut() {
                            *semi = Some(Default::default());
                        }
                        stmts
                    }
                    (_, call) => statement(call),
                }
            }
            // R34: `abort` rather than `panic!`, which ends the program with
            // another exit code.
            ExprKind::Assert { cond, message } => match self.decided(cond)? {
                Some(true) => Vec::new(),
                _ => {
                    let holds = self.cond(cond)?;
                    let message = rust::string(message);
                    self.helper(super::Helper::AssertHolds);
                    statement(rust::call("assert_holds", [holds, message]))
                }
            },
            // R35: a statement expression as a statement is a block.
            ExprKind::Block { stmts, value } => {
                let mut body = self.stmts(stmts)?;
                if let Some(value) = value {
                    body.extend(self.effect(value)?);
                }
                match body.is_empty() {
                    true => Vec::new(),
                    false => {
                        let body = rust::block(body);
                        vec![parse_quote!(#body)]
                    }
                }
            }
            ExprKind::Init { .. } => {
                return unsupported("a first value without its declaration", expr.at);
            }
            // A value that is not used, as in `(void)argc;`.
            _ if expr.is_pure() => Vec::new(),
            _ => statement(self.expr(expr)?),
        })
    }
}

/// A step on the way from a variable to the memory a pointer points to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Var(VarId),
    Field(usize),
    /// What the pointer held there points to.
    Target,
}

/// The memory a pointer expression points into, as far as the expression
/// names it: `a` for `a + 1` and `&a[i]`, `s.x` for `&s.x`, what `p` points
/// to for `p`. Two of them overlap when one starts with the other.
fn pointed(pointer: &Expr) -> Option<Vec<Part>> {
    match &pointer.kind {
        ExprKind::Cast(inner) if inner.ty.is_pointer() => pointed(inner),
        ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right) => {
            pointed(if left.ty.is_pointer() { left } else { right })
        }
        ExprKind::Decay(place) | ExprKind::AddrOf(place) => place_parts(place),
        ExprKind::Var(_) | ExprKind::Member { .. } => {
            let mut parts = place_parts(pointer)?;
            parts.push(Part::Target);
            Some(parts)
        }
        _ => None,
    }
}

/// The expression under conversions that keep every value of an `int`: to
/// an index or to 64 bits.
fn counted(index: &Expr) -> &Expr {
    match &index.kind {
        ExprKind::Cast(inner)
            if index.ty == Type::Index || matches!(index.ty.int(), Some((64, _))) =>
        {
            counted(inner)
        }
        _ => index,
    }
}

/// Whether two expressions name the same place: a variable, a member of
/// such a place, or what a pointer there points to.
fn same_place(one: &Expr, other: &Expr) -> bool {
    match (&one.kind, &other.kind) {
        (ExprKind::Var(one), ExprKind::Var(other)) => one == other,
        (
            ExprKind::Member {
                base: one,
                field: one_field,
                arrow: one_arrow,
                ..
            },
            ExprKind::Member {
                base: other,
                field: other_field,
                arrow: other_arrow,
                ..
            },
        ) => one_field == other_field && one_arrow == other_arrow && same_place(one, other),
        (ExprKind::Decay(one), ExprKind::Decay(other))
        | (ExprKind::Deref(one), ExprKind::Deref(other)) => same_place(one, other),
        (ExprKind::Cast(one), ExprKind::Cast(other)) => same_place(one, other),
        _ => false,
    }
}

fn place_parts(place: &Expr) -> Option<Vec<Part>> {
    match &place.kind {
        ExprKind::Var(var) => Some(vec![Part::Var(*var)]),
        ExprKind::Member {
            base, field, arrow, ..
        } => {
            let mut parts = match arrow {
                true => pointed(base)?,
                false => place_parts(base)?,
            };
            parts.push(Part::Field(*field));
            Some(parts)
        }
        // Any element stands for the array.
        ExprKind::Index(base, _) => pointed(base),
        ExprKind::Deref(pointer) => pointed(pointer),
        _ => None,
    }
}

/// The number under a conversion to an index, if it is signed and not a
/// constant, so that it may be negative when the program runs.
fn signed_offset(index: &Expr) -> Option<&Expr> {
    match &index.kind {
        ExprKind::Cast(inner)
            if index.ty == Type::Index
                && matches!(inner.ty.int(), Some((_, true)))
                && !matches!(inner.kind, ExprKind::Int(_) | ExprKind::Char(_)) =>
        {
            Some(inner)
        }
        _ => None,
    }
}

/// Whether the two expressions read the same places and constants in the
/// same way, so that they have one value. Neither has an effect.
fn calls_realloc(expr: &Expr) -> bool {
    matches!(&expr.kind, ExprKind::Call(callee, _) if callee.function() == Some("realloc"))
        || expr.children().into_iter().any(calls_realloc)
}

fn same_value(left: &Expr, right: &Expr) -> bool {
    left.ty == right.ty
        && match (&left.kind, &right.kind) {
            (ExprKind::Var(one), ExprKind::Var(other)) => one == other,
            (ExprKind::Int(one), ExprKind::Int(other)) => one == other,
            (ExprKind::Char(one), ExprKind::Char(other)) => one == other,
            (ExprKind::Enumerator(one), ExprKind::Enumerator(other)) => one == other,
            (
                ExprKind::Member {
                    base: one,
                    record,
                    field,
                    ..
                },
                ExprKind::Member {
                    base: other,
                    record: other_record,
                    field: other_field,
                    ..
                },
            ) => record == other_record && field == other_field && same_value(one, other),
            (ExprKind::Deref(one), ExprKind::Deref(other))
            | (ExprKind::Decay(one), ExprKind::Decay(other))
            | (ExprKind::Cast(one), ExprKind::Cast(other))
            | (ExprKind::Bits(one), ExprKind::Bits(other)) => same_value(one, other),
            (ExprKind::Unary(op, one), ExprKind::Unary(other_op, other)) => {
                op == other_op && same_value(one, other)
            }
            (ExprKind::Index(one, at), ExprKind::Index(other, other_at)) => {
                same_value(one, other) && same_value(at, other_at)
            }
            (ExprKind::Binary(op, one, two), ExprKind::Binary(other_op, other, other_two)) => {
                op == other_op && same_value(one, other) && same_value(two, other_two)
            }
            _ => false,
        }
}

/// The integer that `value` converted, where converting `value` to `to`
/// gives that integer back whatever it is: `value` is at least as wide.
fn round_trip<'e>(value: &'e Expr, to: &Type) -> Option<&'e Expr> {
    match (&value.kind, to, &value.ty) {
        (ExprKind::Cast(original), Type::Int { bits, .. }, Type::Int { bits: middle, .. })
            if original.ty == *to && middle >= bits =>
        {
            Some(original)
        }
        _ => None,
    }
}

/// The count of a shift without a conversion that only widens it: a shift
/// takes its count in any integer type.
fn shift_count(op: BinOp, count: &Expr) -> &Expr {
    match &count.kind {
        ExprKind::Cast(inner)
            if matches!(op, BinOp::Shl | BinOp::Shr)
                && matches!(inner.ty, Type::Int { .. } | Type::Index)
                && widens(&inner.ty, &count.ty) =>
        {
            inner
        }
        _ => count,
    }
}

/// Whether converting an integer of type `from` to `to` keeps every value.
/// A `bool` is left out: Rust converts it to an integer only.
fn widens(from: &Type, to: &Type) -> bool {
    match (from, from.int(), to.int()) {
        (Type::Bool, ..) => false,
        (_, Some((bits, signed)), Some((wide, wide_signed))) => {
            (wide > bits && (wide_signed || !signed)) || (wide == bits && wide_signed == signed)
        }
        _ => false,
    }
}

/// A floating-point literal of type `ty`: an `f32` as the shortest text of
/// its own bits.
fn float(value: f64, ty: &Type, typed: bool) -> syn::Expr {
    match ty {
        Type::Float { bits: 32 } => rust::single(value, float_suffix(ty, typed)),
        _ => rust::float(value, float_suffix(ty, typed)),
    }
}

fn float_suffix(ty: &Type, typed: bool) -> &'static str {
    match ty {
        Type::Float { bits: 32 } if !typed => "f32",
        _ => "",
    }
}
