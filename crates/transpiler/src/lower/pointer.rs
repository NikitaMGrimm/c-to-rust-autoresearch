//! Rules for pointers. A pointer's Rust type comes from the facts about
//! its slot, and every use of a pointer converts between two such types.

use super::{Lower, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Type, VarKind};
use crate::facts::{ALLOCATORS, Slot, is_argv, place_slot, to_pointer};
use crate::rust;
use syn::parse_quote;

/// R59 grows an array only in place, in the slot that held it.
pub(super) const REALLOC_ELSEWHERE: &str =
    "`realloc` whose result is not stored back in the pointer it grows: no rule yet";

/// Whether the callee is the library's `getenv`.
pub(super) fn is_getenv(program: &crate::c::Program, callee: &Expr) -> bool {
    callee.function() == Some("getenv") && program.function("getenv").is_none()
}

/// How Rust holds what a slot's pointers point to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Repr {
    /// `Box` or `Vec` rather than a borrow.
    pub owned: bool,
    /// A `Vec` or a slice rather than one element.
    pub many: bool,
    /// For a borrow: `&mut` rather than `&`.
    pub mutable: bool,
    /// Wrapped in `Option`.
    pub nullable: bool,
    /// A `&str`: the pointer only ever holds text that is passed on whole.
    pub text: bool,
    /// For a pointer to pointers: the slot behind it, which says what its
    /// elements are.
    pub behind: Option<Slot>,
}

impl Repr {
    /// Text that is there, for a use that reads it.
    pub(super) const TEXT: Repr = Repr {
        owned: false,
        many: false,
        mutable: false,
        nullable: false,
        text: true,
        behind: None,
    };

    /// Bytes that are there, for a use that reads them.
    pub(super) const BYTES: Repr = Repr {
        owned: false,
        many: true,
        mutable: false,
        nullable: false,
        text: false,
        behind: None,
    };

    /// Bytes that are there, for a use that stores to them.
    pub(super) const BYTES_WRITTEN: Repr = Repr {
        mutable: true,
        ..Repr::BYTES
    };
}

impl Lower<'_> {
    pub(super) fn slot_name(&self, slot: Slot) -> String {
        let program = self.program;
        match slot {
            Slot::Var(var) => format!("the pointer `{}`", program.vars[var].name),
            Slot::Field(record, field) => {
                format!("the field `{}`", program.records[record].fields[field].name)
            }
            Slot::Return(function) => {
                format!("the result of `{}`", program.functions[function].name)
            }
            Slot::Behind(_) => match slot.outer() {
                Some(outer) => format!("the place behind {}", self.slot_name(outer)),
                None => "a place behind a pointer".to_string(),
            },
        }
    }

    pub(super) fn repr(&self, slot: Slot, at: u32) -> R<Repr> {
        let pointer = self.facts.pointer(slot);
        if let Some(why) = pointer.conflict {
            return unsupported(format!("{}: {why}", self.slot_name(slot)), at);
        }
        let text = !(pointer.bytes || pointer.written || pointer.owned)
            && self.slot_c_type(slot).element() == Some(&Type::Char);
        // R39: a record or a global owns what its pointers hold, which
        // makes them bytes.
        let kept = match slot {
            Slot::Var(var) => self.program.vars[var].kind.is_global(),
            Slot::Field(..) | Slot::Behind(_) => true,
            Slot::Return(_) => false,
        };
        if pointer.environment && kept {
            return unsupported(
                format!(
                    "{}: a variable of the environment kept in a record or a global: no rule yet",
                    self.slot_name(slot)
                ),
                at,
            );
        }
        // R39: the environment's text is not the program's to change, and
        // a `String` has no bytes to walk.
        if pointer.environment && !text {
            return unsupported(
                format!(
                    "{}: a variable of the environment that is stored through, moved, freed or read byte by byte: no rule yet",
                    self.slot_name(slot)
                ),
                at,
            );
        }
        Ok(Repr {
            owned: pointer.owned || pointer.environment,
            // A string literal is an array to the analysis; a `&str` is
            // one value.
            many: pointer.many && !text,
            mutable: pointer.written,
            nullable: pointer.nullable,
            text,
            behind: slot.behind().filter(|_| to_pointer(self.slot_c_type(slot))),
        })
    }

    fn slot_c_type(&self, slot: Slot) -> &Type {
        crate::facts::slot_type(self.program, slot)
    }

    /// Whether the pointer expression reads a slot that holds text.
    pub(super) fn is_text(&self, pointer: &Expr) -> bool {
        match &pointer.kind {
            ExprKind::Cast(inner) if inner.ty.is_pointer() => self.is_text(inner),
            ExprKind::Call(callee, _) if is_getenv(self.program, callee) => true,
            ExprKind::Var(_)
            | ExprKind::Member { .. }
            | ExprKind::Deref(_)
            | ExprKind::Index(..)
            | ExprKind::Call(..)
                if !self.is_argument(pointer) =>
            {
                self.slot_of(pointer)
                    .and_then(|slot| self.repr(slot, pointer.at))
                    .is_ok_and(|repr| repr.text)
            }
            // R21: text from the index where a number ends in it.
            ExprKind::Binary(BinOp::Add, text, index) if index.ty == Type::Index => {
                self.is_text(text) || self.is_argument(text)
            }
            _ => false,
        }
    }

    /// The parts of `text + index`, where a number ends in text: the
    /// whole text and the index.
    pub(super) fn text_at(&mut self, text: &Expr, index: &Expr) -> R<(syn::Expr, syn::Expr)> {
        let text = match self.is_argument(text) {
            true => self.argument(text)?,
            false => self.pointer(text, Repr::TEXT)?,
        };
        Ok((text, self.expr(index)?))
    }

    /// R7, R8: the type of a slot: `Box<T>` or `Vec<T>` for an owner, `&T`,
    /// `&mut T` or a slice for a borrow, in `Option` when it may be NULL.
    pub(super) fn slot_type(
        &self,
        slot: Slot,
        pointer: &Type,
        lifetime: Option<&syn::Lifetime>,
        at: u32,
    ) -> R<syn::Type> {
        let repr = self.repr(slot, at)?;
        if let Some(function) = pointer.function() {
            let ty = self.fn_type(function, at)?;
            return Ok(match repr.nullable {
                true => parse_quote!(Option<#ty>),
                false => ty,
            });
        }
        let element = match pointer.element() {
            Some(Type::Void) => return unsupported("a `void` pointer", at),
            Some(Type::Pointer(_)) => return self.behind_type(slot, pointer, lifetime, at),
            Some(element) => self.ty(element, at)?,
            None => return unsupported("a pointer without a type", at),
        };
        let mutability: Option<syn::Token![mut]> = repr.mutable.then(Default::default);
        let ty: syn::Type = match (repr.owned, repr.many) {
            _ if repr.text && repr.owned => parse_quote!(String),
            _ if repr.text => parse_quote!(& #lifetime str),
            (true, false) => parse_quote!(Box<#element>),
            (true, true) => parse_quote!(Vec<#element>),
            (false, false) => parse_quote!(& #lifetime #mutability #element),
            (false, true) => parse_quote!(& #lifetime #mutability [#element]),
        };
        Ok(match repr.nullable {
            true => parse_quote!(Option<#ty>),
            false => ty,
        })
    }

    /// R65: a parameter `T **pp` that names one place holding a pointer:
    /// a borrow of what the slot behind it is, `&mut Option<Box<T>>` where
    /// the function stores to the place or through it.
    fn behind_type(
        &self,
        slot: Slot,
        pointer: &Type,
        lifetime: Option<&syn::Lifetime>,
        at: u32,
    ) -> R<syn::Type> {
        let Some(element) = pointer.element() else {
            return unsupported("a pointer without a type", at);
        };
        match element.element() {
            Some(Type::Void) => return unsupported("a `void **`", at),
            Some(Type::Pointer(_)) => {
                return unsupported("a pointer to a pointer to a pointer", at);
            }
            _ if element.function().is_some() => {
                return unsupported("a pointer to a pointer to a function", at);
            }
            _ => {}
        }
        let param = matches!(slot, Slot::Var(var) if self.program.vars[var].kind == VarKind::Param);
        let repr = self.repr(slot, at)?;
        // R67: an array of pointers that the pointer owns, as a local, or
        // borrows, as a parameter.
        let array = repr.many && !repr.nullable && !repr.text && (repr.owned || param);
        if !array && (!param || repr.owned || repr.many || repr.nullable)
            || !to_pointer(pointer)
            || !matches!(slot, Slot::Var(_))
        {
            return unsupported("a pointer to a pointer", at);
        }
        let Some(behind) = slot.behind() else {
            return unsupported("a pointer to a pointer to a pointer", at);
        };
        // A borrow behind a borrow needs two lifetimes that the
        // translation does not know.
        // R67: a slice of text that nothing stores to is `&[&str]`.
        let held = self.repr(behind, at)?;
        let text = held.text && repr.many && !repr.owned && !repr.mutable;
        if !held.owned && !text {
            return unsupported(
                format!(
                    "{}: a borrow behind a pointer: no rule yet",
                    self.slot_name(behind)
                ),
                at,
            );
        }
        let held = self.slot_type(behind, element, None, at)?;
        let mutability: Option<syn::Token![mut]> = repr.mutable.then(Default::default);
        Ok(match (repr.many, repr.owned) {
            (true, true) => parse_quote!(Vec<#held>),
            (true, false) => parse_quote!(& #lifetime #mutability [#held]),
            _ => parse_quote!(& #lifetime #mutability #held),
        })
    }

    /// Whether the expression is `argv[i]`, one of `main`'s arguments.
    pub(super) fn is_argument(&self, text: &Expr) -> bool {
        matches!(&text.uncast().kind, ExprKind::Index(base, _) if is_argv(self.program, base))
    }

    /// `argv[i]` as `args[i]`, a `String`.
    pub(super) fn argument(&mut self, text: &Expr) -> R<syn::Expr> {
        match &text.uncast().kind {
            ExprKind::Index(_, index) => {
                Ok(rust::index(rust::name("args"), self.index_value(index)?))
            }
            _ => unsupported("a program argument with no rule", text.at),
        }
    }

    /// The slot whose pointer the expression reads.
    fn slot_of(&self, pointer: &Expr) -> R<Slot> {
        if let ExprKind::Call(callee, _) = &pointer.kind {
            let name = callee.function().unwrap_or_default();
            return match self.program.function(name) {
                Some(function) => Ok(Slot::Return(function)),
                None if name == "__errno_location" => {
                    unsupported("`errno` as a place: no rule yet", pointer.at)
                }
                None => unsupported(format!("a pointer from the library's `{name}`"), pointer.at),
            };
        }
        match place_slot(pointer) {
            Some(slot) => Ok(slot),
            None => unsupported("a pointer in a place with no rule", pointer.at),
        }
    }

    /// The Rust value a slot expression reads: the variable, the field or
    /// the call.
    fn slot_value(&mut self, pointer: &Expr, write: bool) -> R<syn::Expr> {
        match &pointer.kind {
            ExprKind::Call(callee, args) => self.call(pointer, callee, args),
            _ => self.place(pointer, write),
        }
    }

    /// R7, R8: a pointer expression as the Rust value that a slot of
    /// representation `to` holds.
    pub(super) fn pointer(&mut self, pointer: &Expr, to: Repr) -> R<syn::Expr> {
        let at = pointer.at;
        if pointer.ty.function().is_some() {
            return self.function_value(pointer, to.nullable);
        }
        let wrap = |value: syn::Expr| match to.nullable {
            true => rust::call("Some", [value]),
            false => value,
        };
        match &pointer.kind {
            ExprKind::Null => match to.nullable {
                true => Ok(parse_quote!(None)),
                false => unsupported("NULL for a pointer that the analysis found never NULL", at),
            },
            ExprKind::Cast(inner) => {
                if let ExprKind::Call(callee, args) = &inner.kind
                    && let Some(name) = callee.function()
                    && ALLOCATORS.contains(&name)
                {
                    return Ok(wrap(self.allocate(name, args, pointer, to)?));
                }
                if let ExprKind::Call(callee, _) = &inner.kind
                    && callee.function() == Some("realloc")
                {
                    return unsupported(REALLOC_ELSEWHERE, at);
                }
                self.same_element(pointer, inner)?;
                self.pointer(inner, to)
            }
            ExprKind::Cond(cond, then, other) => match self.decided(cond)? {
                Some(true) => self.pointer(then, to),
                Some(false) => self.pointer(other, to),
                None => {
                    let cond = self.cond(cond)?;
                    let (then, other) = (self.pointer(then, to)?, self.pointer(other, to)?);
                    Ok(parse_quote!(if #cond { #then } else { #other }))
                }
            },
            ExprKind::Call(callee, args) if is_getenv(self.program, callee) => {
                let from = Repr {
                    owned: true,
                    nullable: true,
                    ..Repr::TEXT
                };
                let value = rust::method(self.environment(args, at)?, "ok", []);
                self.adapt(value, from, to, true, at)
            }
            ExprKind::Var(_)
            | ExprKind::Member { .. }
            | ExprKind::Deref(_)
            | ExprKind::Index(..)
            | ExprKind::Call(..)
                if !self.is_argument(pointer) =>
            {
                let from = self.repr(self.slot_of(pointer)?, at)?;
                let value = self.slot_value(pointer, to.mutable || to.owned)?;
                let result = matches!(pointer.kind, ExprKind::Call(..));
                let value = match from.owned && to.owned {
                    true => {
                        self.restored_after(pointer)?;
                        self.moved(pointer, value, from)?
                    }
                    false => value,
                };
                self.adapt(value, from, to, result, at)
            }
            ExprKind::Index(..) if self.is_argument(pointer) => {
                if to.owned {
                    return unsupported("a program argument for a pointer that owns", at);
                }
                if !to.text {
                    return unsupported("a program argument read byte by byte: no rule yet", at);
                }
                let argument = self.argument(pointer)?;
                Ok(match (to.nullable, argument) {
                    // `argv[argc]` is NULL, and there `get` finds nothing.
                    (true, syn::Expr::Index(syn::ExprIndex { expr, index, .. })) => {
                        parse_quote!(#expr.get(#index).map(String::as_str))
                    }
                    (true, _) => return unsupported("a program argument with no rule", at),
                    (false, argument) => rust::borrow(false, argument),
                })
            }
            ExprKind::AddrOf(place) => {
                if to.owned {
                    return unsupported("the address of a variable for a pointer that owns", at);
                }
                if !to.many {
                    return Ok(wrap(rust::borrow(
                        to.mutable,
                        self.place(place, to.mutable)?,
                    )));
                }
                // `&a[i]` is the rest of the array from `i`.
                if let ExprKind::Index(base, index) = &place.kind {
                    let rest = rust::from(self.index_value(index)?);
                    let slice = rust::index(self.slice(base, to.mutable)?, rest);
                    return Ok(wrap(rust::borrow(to.mutable, slice)));
                }
                let one = rust::borrow(to.mutable, self.place(place, to.mutable)?);
                Ok(wrap(match to.mutable {
                    true => rust::call("std::slice::from_mut", [one]),
                    false => rust::call("std::slice::from_ref", [one]),
                }))
            }
            ExprKind::Decay(array) => {
                if to.owned && !(to.text && matches!(array.kind, ExprKind::Str(_))) {
                    return unsupported("an array for a pointer that owns", at);
                }
                if let ExprKind::Str(bytes) = &array.kind {
                    if !to.text {
                        // The program reads the bytes one by one, up to
                        // the NUL that C puts after them.
                        let mut bytes = bytes.clone();
                        bytes.push(0);
                        let literal = syn::LitByteStr::new(&bytes, proc_macro2::Span::call_site());
                        return Ok(match to.nullable {
                            // An array becomes a slice by itself only
                            // outside `Some`.
                            true => parse_quote!(Some(&#literal[..])),
                            false => parse_quote!(#literal),
                        });
                    }
                    // Text ends at its first NUL, as every reader of it
                    // in C finds.
                    let text = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
                    return match std::str::from_utf8(text) {
                        // R39: a slot that also holds the environment's
                        // text owns it.
                        Ok(text) if to.owned => {
                            Ok(wrap(rust::call("String::from", [rust::string(text)])))
                        }
                        Ok(text) => Ok(wrap(rust::string(text))),
                        Err(_) => unsupported("a string literal that is not UTF-8", at),
                    };
                }
                let array = self.place(array, to.mutable)?;
                Ok(wrap(rust::borrow(
                    to.mutable,
                    match (to.many, to.nullable) {
                        (false, _) => rust::index(array, rust::int(0, "")),
                        // An array becomes a slice by itself only outside `Some`.
                        (true, true) => rust::index(array, parse_quote!(..)),
                        (true, false) => array,
                    },
                )))
            }
            ExprKind::Binary(BinOp::Add, text, index) if to.text && self.is_text(pointer) => {
                // A `&str` cannot start inside a character. The end of a
                // number never is, and other indices have no rule.
                if !matches!(index.kind, ExprKind::Var(_)) {
                    return unsupported("text from an index that is not the end of a number", at);
                }
                let (text, index) = self.text_at(text, index)?;
                Ok(wrap(parse_quote!(&#text[#index..])))
            }
            ExprKind::Binary(BinOp::Add, left, right) if to.many && !to.owned => {
                let (base, offset) = match left.ty.is_pointer() {
                    true => (left, right),
                    false => (right, left),
                };
                let rest = rust::from(self.index_value(offset)?);
                let slice = rust::index(self.slice(base, to.mutable)?, rest);
                Ok(wrap(rust::borrow(to.mutable, slice)))
            }
            _ => unsupported("a pointer expression with no rule", at),
        }
    }

    /// R39: `getenv(name)` as `std::env::var(name)`, whose error stands
    /// for NULL.
    pub(super) fn environment(&mut self, args: &[Expr], at: u32) -> R<syn::Expr> {
        match args {
            [name] => {
                // `text_of` gives a `Cow<str>`, which `var` takes as a `str`.
                let name = match self.reads_bytes(name) {
                    true => rust::borrow(false, rust::unary("*", self.text(name, false)?)),
                    false => self.text(name, true)?,
                };
                Ok(rust::call("std::env::var", [name]))
            }
            _ => unsupported("`getenv` with other than one argument", at),
        }
    }

    /// R65: a move out of `*pp` leaves `None` behind, where C still holds
    /// the pointer; only a store to `*pp` in the next statement, which C
    /// makes before any read, gives the place its value back.
    fn restored_after(&self, pointer: &Expr) -> R<()> {
        if let ExprKind::Deref(behind) = &pointer.kind
            && !matches!(behind.kind, ExprKind::Var(var) if self.restored == Some(var))
        {
            return unsupported(
                "a move out of the place behind a pointer that something may read before it is stored to: no rule yet",
                pointer.at,
            );
        }
        // R67: a move out of an element leaves `None` in it, where C
        // still holds the pointer.
        if let ExprKind::Index(..) = &pointer.kind
            && !self.element_restored(pointer)
        {
            return unsupported(
                "a move out of an element of an array of pointers that the next statement does not store to: no rule yet",
                pointer.at,
            );
        }
        Ok(())
    }

    /// Whether the statement after the one being written stores to the
    /// element `a[i]` again, before anything reads it.
    fn element_restored(&self, element: &Expr) -> bool {
        self.restored_element
            .as_ref()
            .is_some_and(|restored| super::stmt::same_element(restored, element))
    }

    /// What an owner holds, moved out of it. A record or `Globals` stays in
    /// use, so what it holds is replaced by an empty value.
    fn moved(&self, pointer: &Expr, value: syn::Expr, from: Repr) -> R<syn::Expr> {
        let held = match pointer.kind {
            ExprKind::Member { .. } | ExprKind::Deref(_) | ExprKind::Index(..)
                if !self.is_argument(pointer) =>
            {
                true
            }
            ExprKind::Var(var) => self.program.vars[var].kind.is_global(),
            _ => false,
        };
        if let ExprKind::Member { record, field, .. } = pointer.kind
            && self.twice.contains(&(record, field))
        {
            return unsupported(
                "a move out of a field that is read again before it is stored to: no rule yet",
                pointer.at,
            );
        }
        Ok(match (held, from.nullable, from.many) {
            (false, ..) => value,
            (true, true, _) => rust::method(value, "take", []),
            (true, false, true) => rust::call("std::mem::take", [rust::borrow(true, value)]),
            (true, false, false) => {
                return unsupported(
                    "a move out of a field or a global that is never NULL: no rule yet",
                    pointer.at,
                );
            }
        })
    }

    /// A pointer as an argument of a function whose parameter is a borrow
    /// written out, not a generic. Rust reborrows a `&mut` variable that is
    /// passed to such a parameter, as `&mut` or as `&`, so it is passed by
    /// its name.
    pub(super) fn argument_pointer(&mut self, pointer: &Expr, to: Repr) -> R<syn::Expr> {
        if let ExprKind::Var(var) = pointer.kind
            && !to.owned
            && !to.nullable
        {
            let from = self.repr(Slot::Var(var), pointer.at)?;
            let local = matches!(self.program.vars[var].kind, VarKind::Local | VarKind::Param);
            // The slots behind two pointers are one where one is passed
            // for the other.
            let from = Repr {
                behind: None,
                ..from
            };
            let to_same = Repr { behind: None, ..to };
            if (from == to_same && to.mutable)
                || (local
                    && from.mutable
                    && Repr {
                        mutable: false,
                        ..from
                    } == to_same)
            {
                return self.place(pointer, to.mutable);
            }
        }
        self.pointer(pointer, to)
    }

    /// A cast between pointers keeps the element type, apart from `void`.
    fn same_element(&self, cast: &Expr, inner: &Expr) -> R<()> {
        if !inner.ty.is_pointer() {
            return unsupported("a pointer made from a number", cast.at);
        }
        if (cast.ty.function().is_some() || inner.ty.function().is_some()) && cast.ty != inner.ty {
            return unsupported("a cast of a pointer to a function", cast.at);
        }
        if (to_pointer(&cast.ty) || to_pointer(&inner.ty)) && cast.ty != inner.ty {
            return unsupported("a cast to or from a pointer to a pointer", cast.at);
        }
        match (cast.ty.element(), inner.ty.element()) {
            (Some(Type::Void), _) | (_, Some(Type::Void)) => Ok(()),
            (to, from) if to == from => Ok(()),
            _ => unsupported("a cast between pointers to different types", cast.at),
        }
    }

    /// Converts the value of a slot of representation `from` for one of
    /// `to`: a move between owners, a borrow of an owner, or a reborrow.
    /// A call's `result` is no place, so a borrow in it is passed on as it
    /// is.
    fn adapt(&self, value: syn::Expr, from: Repr, to: Repr, result: bool, at: u32) -> R<syn::Expr> {
        let some = |value| rust::call("Some", [value]);
        let unwrap = |value| rust::method(value, "unwrap", []);
        let as_deref = |value| match to.mutable {
            true => rust::method(value, "as_deref_mut", []),
            false => rust::method(value, "as_deref", []),
        };
        if from.many != to.many {
            return unsupported(
                "a pointer to one element where an array is expected, or the reverse",
                at,
            );
        }
        Ok(match (from.owned, to.owned) {
            (true, true) => match (from.nullable, to.nullable) {
                (false, true) => some(value),
                (true, false) => unwrap(value),
                _ => value,
            },
            (false, true) => {
                return unsupported(
                    "a borrowed pointer where the program frees or keeps the memory",
                    at,
                );
            }
            (true, false) => match (from.nullable, to.nullable) {
                (true, true) => as_deref(value),
                (true, false) => unwrap(as_deref(value)),
                // A `Box` or `Vec` becomes a borrow by itself only outside
                // `Some`.
                (false, true) => some(rust::borrow(
                    to.mutable,
                    match from.many {
                        true => rust::index(value, parse_quote!(..)),
                        false => rust::unary("*", value),
                    },
                )),
                (false, false) => rust::borrow(to.mutable, value),
            },
            (false, false) => {
                if to.mutable && !from.mutable {
                    return unsupported("a store through a pointer that only reads", at);
                }
                let value = match (from.mutable, from.nullable) {
                    // `&T` and `Option<&T>` are copied.
                    (false, _) => value,
                    _ if result && from.mutable == to.mutable => value,
                    // `as_deref` would borrow from the result, which ends
                    // with its statement.
                    (true, true) if result => parse_quote!(#value.map(|found| &*found)),
                    (true, true) => as_deref(value),
                    (true, false) => rust::borrow(to.mutable, rust::unary("*", value)),
                };
                match (from.nullable, to.nullable) {
                    (false, true) => some(value),
                    (true, false) => unwrap(value),
                    _ => value,
                }
            }
        })
    }

    /// What the pointer points to, as a place: `*p`, or `p` when a field
    /// access follows, which dereferences by itself.
    pub(super) fn pointee(&mut self, pointer: &Expr, write: bool, field: bool) -> R<syn::Expr> {
        match &pointer.kind {
            ExprKind::AddrOf(place) => self.place(place, write),
            ExprKind::Cast(inner) => {
                self.same_element(pointer, inner)?;
                self.pointee(inner, write, field)
            }
            ExprKind::Var(_)
            | ExprKind::Member { .. }
            | ExprKind::Deref(_)
            | ExprKind::Index(..)
            | ExprKind::Call(..)
                if !self.is_argument(pointer) =>
            {
                let repr = match &pointer.kind {
                    ExprKind::Call(callee, _) if is_getenv(self.program, callee) => Repr::TEXT,
                    _ => self.repr(self.slot_of(pointer)?, pointer.at)?,
                };
                if repr.many {
                    return Ok(rust::index(self.slice(pointer, write)?, rust::int(0, "")));
                }
                if repr.text {
                    if write {
                        return unsupported("a store into text", pointer.at);
                    }
                    // The first byte of the text, or the NUL that ends it
                    // in C.
                    let text = self.pointer(pointer, Repr::TEXT)?;
                    let first = rust::method(rust::method(text, "bytes", []), "next", []);
                    return Ok(rust::method(first, "unwrap_or", [rust::int(0, "")]));
                }
                let value = self.slot_value(pointer, write)?;
                // `p = p->next` for a borrow that may be NULL and that is
                // stored through: the borrow goes on, and `p` holds
                // nothing until the assignment gives it the next one. A
                // reborrow of `p` would still be in use when `p` is
                // assigned.
                let walks = matches!(pointer.kind, ExprKind::Var(var) if self.walking == Some(var))
                    && repr.nullable
                    && repr.mutable
                    && !repr.owned;
                let value = match walks {
                    true => rust::method(rust::method(value, "take", []), "unwrap", []),
                    false => self.unwrapped(value, repr, write),
                };
                Ok(if field {
                    value
                } else {
                    rust::unary("*", value)
                })
            }
            // R21: the byte of text at an index, or the NUL that ends it
            // in C.
            ExprKind::Binary(BinOp::Add, text, index) if self.is_text(pointer) => {
                if write {
                    return unsupported("a store into text", pointer.at);
                }
                let (text, index) = self.text_at(text, index)?;
                Ok(parse_quote!(#text.as_bytes().get(#index).copied().unwrap_or(0)))
            }
            ExprKind::Decay(_) | ExprKind::Binary(BinOp::Add, ..) => {
                Ok(rust::index(self.slice(pointer, write)?, rust::int(0, "")))
            }
            _ => unsupported("a pointer expression with no rule", pointer.at),
        }
    }

    /// The value of a slot without its `Option`, to reach what it points to.
    fn unwrapped(&self, value: syn::Expr, repr: Repr, write: bool) -> syn::Expr {
        match (repr.nullable, repr.owned || repr.mutable) {
            (false, _) => value,
            (true, false) => rust::method(value, "unwrap", []),
            (true, true) => {
                let borrowed =
                    rust::method(value, if write { "as_deref_mut" } else { "as_deref" }, []);
                rust::method(borrowed, "unwrap", [])
            }
        }
    }

    /// The array a pointer points into, as a place that can be indexed.
    pub(super) fn slice(&mut self, pointer: &Expr, write: bool) -> R<syn::Expr> {
        match &pointer.kind {
            ExprKind::Decay(array) => match array.kind {
                ExprKind::Str(_) => {
                    unsupported("a string literal as an array: no rule yet", pointer.at)
                }
                _ => self.place(array, write),
            },
            ExprKind::Cast(inner) => {
                self.same_element(pointer, inner)?;
                self.slice(inner, write)
            }
            ExprKind::Binary(BinOp::Add, left, right) => {
                let (base, offset) = match left.ty.is_pointer() {
                    true => (left, right),
                    false => (right, left),
                };
                let rest = rust::from(self.index_value(offset)?);
                Ok(rust::index(self.slice(base, write)?, rest))
            }
            ExprKind::AddrOf(place) => match &place.kind {
                ExprKind::Index(base, index) => {
                    let rest = rust::from(self.index_value(index)?);
                    Ok(rust::index(self.slice(base, write)?, rest))
                }
                _ => unsupported("the address of one variable used as an array", pointer.at),
            },
            ExprKind::Var(_)
            | ExprKind::Member { .. }
            | ExprKind::Deref(_)
            | ExprKind::Index(..)
            | ExprKind::Call(..)
                if !self.is_argument(pointer) =>
            {
                let repr = self.repr(self.slot_of(pointer)?, pointer.at)?;
                if !repr.many {
                    return unsupported("a pointer to one element used as an array", pointer.at);
                }
                let value = self.slot_value(pointer, write)?;
                Ok(self.unwrapped(value, repr, write))
            }
            _ => unsupported("a pointer expression with no rule", pointer.at),
        }
    }

    /// Whether the pointer expression reads a slot that holds one element.
    pub(super) fn points_to_one(&self, pointer: &Expr) -> bool {
        match &pointer.kind {
            ExprKind::Cast(inner) if inner.ty.is_pointer() => self.points_to_one(inner),
            ExprKind::AddrOf(_) => true,
            ExprKind::Var(_)
            | ExprKind::Member { .. }
            | ExprKind::Deref(_)
            | ExprKind::Index(..)
            | ExprKind::Call(..)
                if !self.is_argument(pointer) =>
            {
                self.slot_of(pointer)
                    .is_ok_and(|slot| !self.facts.pointer(slot).many)
            }
            _ => false,
        }
    }

    /// An index or a count as `usize`.
    pub(super) fn index_value(&mut self, index: &Expr) -> R<syn::Expr> {
        if let Some(index @ 0..) = index.constant(self.program) {
            return Ok(rust::int(index, ""));
        }
        if index.ty == Type::Index {
            return self.expr(index);
        }
        // C widens an index to 64 bits first, which `as usize` does too.
        let index = match &index.kind {
            ExprKind::Cast(inner)
                if matches!(index.ty.int(), Some((64, _))) && inner.ty.int().is_some() =>
            {
                inner
            }
            _ => index,
        };
        Ok(rust::cast(self.expr(index)?, parse_quote!(usize)))
    }

    /// R8: `malloc` and `calloc` as `Box::new` or `vec!`. Allocation does
    /// not fail in Rust, and the memory starts at zero; C leaves `malloc`'s
    /// memory without a value, so any value is right.
    fn allocate(&mut self, name: &str, args: &[Expr], cast: &Expr, to: Repr) -> R<syn::Expr> {
        let program = self.program;
        let at = cast.at;
        let Some(element) = cast.ty.element() else {
            return unsupported("an allocation without a type", at);
        };
        // R67: an array of pointers holds what the slot behind the
        // pointer holds, which is not `Clone` where it owns a record.
        let behind = match element.is_pointer() {
            true => match to.behind {
                Some(behind) if to.many => Some(behind),
                _ => return unsupported("a pointer to a pointer", at),
            },
            false => None,
        };
        let zero = match behind {
            Some(behind) => self.zero_of(element, Some(behind), at)?,
            None => self.zero(element, at)?,
        };
        if !to.owned {
            return unsupported("an allocation for a pointer that does not own", at);
        }
        if !to.many {
            return Ok(rust::call("Box::new", [zero]));
        }
        let Some((size, _)) = element.layout(program) else {
            return unsupported("an allocation of a type without a size", at);
        };
        let size = i128::from(size);
        let is_size = |bytes: &Expr| bytes.constant(program) == Some(size);
        let count = match (name, args) {
            ("calloc", [count, bytes]) if is_size(bytes) => self.index_value(count)?,
            ("malloc", [bytes]) => match (&bytes.uncast().kind, bytes.constant(program)) {
                (_, Some(bytes)) if bytes % size == 0 => rust::int(bytes / size, ""),
                (ExprKind::Binary(BinOp::Mul, count, bytes), _) if is_size(bytes) => {
                    self.index_value(count)?
                }
                (ExprKind::Binary(BinOp::Mul, bytes, count), _) if is_size(bytes) => {
                    self.index_value(count)?
                }
                _ if size == 1 => self.index_value(bytes)?,
                _ => return unsupported("an allocation whose size has no rule", at),
            },
            _ => return unsupported("an allocation whose size has no rule", at),
        };
        Ok(match behind {
            Some(_) => parse_quote!((0..#count).map(|_| #zero).collect::<Vec<_>>()),
            None => parse_quote!(vec![#zero; #count]),
        })
    }

    /// R59: `p = realloc(p, size)` as `p.resize(count, zero)` on the `Vec`
    /// that `p` is. The elements that C keeps are kept; C leaves the new
    /// ones without a value, so zero is right. A slot that may be NULL
    /// gets an empty `Vec` first, as `realloc(NULL, size)` is `malloc`.
    pub(super) fn resize(&mut self, target: &Expr, bytes: &Expr) -> R<syn::Expr> {
        let program = self.program;
        let at = target.at;
        let Some(slot) = place_slot(target) else {
            return unsupported("`realloc` of a pointer expression with no rule", at);
        };
        // A parameter's array belongs to the caller, which still holds it.
        if let ExprKind::Var(var) = target.kind
            && program.vars[var].kind == VarKind::Param
        {
            return unsupported("`realloc` of a parameter's array", at);
        }
        let repr = self.repr(slot, at)?;
        let Some(element) = target.ty.element() else {
            return unsupported("`realloc` of a pointer without a type", at);
        };
        if !(repr.owned && repr.many) || repr.text {
            return unsupported("`realloc` of a pointer that does not own an array", at);
        }
        let Some((size, _)) = element.layout(program) else {
            return unsupported("`realloc` of a type without a size", at);
        };
        let size = i128::from(size);
        // A size that is always the same and not 0 keeps the array. Any
        // other may be 0 when the program runs, for which the reference's
        // library frees the array and returns NULL.
        let fixed = crate::facts::fixed_size(program, bytes).is_some();
        if !fixed && bytes.constant(program).is_some() {
            return unsupported("a constant size of `realloc` that keeps no array", at);
        }
        // A size is computed in 64 bits: a conversion to fewer cuts it,
        // and a product of narrower numbers wraps.
        let in_size = crate::facts::in_size;
        let is_size = |bytes: &Expr| {
            matches!(in_size(bytes).kind, ExprKind::SizeOf(_))
                && bytes.constant(program) == Some(size)
        };
        let whole = in_size(bytes);
        let count = match &whole.kind {
            ExprKind::Binary(BinOp::Mul, count, bytes)
            | ExprKind::Binary(BinOp::Mul, bytes, count)
                if is_size(bytes) =>
            {
                // C multiplies in 64 bits and keeps the low ones: a count
                // of that width may give a small array where Rust's count
                // of elements has none.
                let wide = count.uncast().ty == Type::Index
                    || matches!(count.uncast().ty.int(), Some((64, _)));
                if !fixed && wide {
                    return unsupported(
                        "a count of `realloc` as wide as a size, whose product C may wrap",
                        at,
                    );
                }
                self.index_value(count)?
            }
            _ if is_size(bytes) => rust::int(1, ""),
            _ if size == 1 => self.index_value(bytes)?,
            _ => return unsupported("a size of `realloc` with no rule", at),
        };
        let zero = self.zero(element, at)?;
        let place = self.place(target, true)?;
        let array = match repr.nullable {
            true => rust::method(
                place.clone(),
                "get_or_insert_with",
                [parse_quote!(Vec::new)],
            ),
            false => place.clone(),
        };
        if fixed {
            return Ok(rust::method(array, "resize", [count, zero]));
        }
        if !repr.nullable {
            return unsupported(
                "`realloc` to a size that may be 0 of a pointer that holds no NULL",
                at,
            );
        }
        // The count is found first: it may read the array itself.
        let held = rust::method(place.clone(), "is_some", []);
        let resize = rust::method(array, "resize", [parse_quote!(grown_len), zero]);
        Ok(parse_quote!({
            let grown_len = #count;
            if grown_len == 0 && #held {
                #place = None;
            } else {
                #resize;
            }
        }))
    }

    /// R8: `free(p)` as `drop`. The memory would be freed at the end of the
    /// owner's scope anyway; `drop` keeps C's order and makes a later use
    /// a compile error.
    pub(super) fn free(&mut self, arg: &Expr) -> R<syn::Expr> {
        let pointer = arg.uncast();
        let slot = match place_slot(pointer) {
            Some(slot) => slot,
            None => return unsupported("`free` of a pointer expression with no rule", arg.at),
        };
        let repr = self.repr(slot, arg.at)?;
        if !repr.owned {
            return unsupported("`free` of a pointer that does not own", arg.at);
        }
        let place = self.place(pointer, true)?;
        Ok(rust::call("drop", [self.moved(pointer, place, repr)?]))
    }
}
