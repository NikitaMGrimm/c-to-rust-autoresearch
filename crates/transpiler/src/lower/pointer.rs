//! Rules for pointers. A pointer's Rust type comes from the facts about
//! its slot, and every use of a pointer converts between two such types.

use super::{Lower, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Type};
use crate::facts::{ALLOCATORS, Slot, place_slot};
use crate::rust;
use syn::parse_quote;

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
        }
    }

    pub(super) fn repr(&self, slot: Slot, at: u32) -> R<Repr> {
        let pointer = self.facts.pointer(slot);
        if let Some(why) = pointer.conflict {
            return unsupported(format!("{}: {why}", self.slot_name(slot)), at);
        }
        Ok(Repr {
            owned: pointer.owned,
            many: pointer.many,
            mutable: pointer.written,
            nullable: pointer.nullable,
        })
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
        let element = match pointer.element() {
            Some(Type::Void) => return unsupported("a `void` pointer", at),
            Some(Type::Pointer(_)) => return unsupported("a pointer to a pointer", at),
            Some(element) => self.ty(element, at)?,
            None => return unsupported("a pointer without a type", at),
        };
        let mutability: Option<syn::Token![mut]> = repr.mutable.then(Default::default);
        let ty: syn::Type = match (repr.owned, repr.many) {
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

    /// The slot whose pointer the expression reads.
    fn slot_of(&self, pointer: &Expr) -> R<Slot> {
        if let ExprKind::Call(callee, _) = &pointer.kind {
            let name = callee.function().unwrap_or_default();
            return match self.program.function(name) {
                Some(function) => Ok(Slot::Return(function)),
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
            ExprKind::Var(_) | ExprKind::Member { .. } | ExprKind::Call(..) => {
                let from = self.repr(self.slot_of(pointer)?, at)?;
                let value = self.slot_value(pointer, to.mutable || to.owned)?;
                let result = matches!(pointer.kind, ExprKind::Call(..));
                let value = match from.owned && to.owned {
                    true => self.moved(pointer, value, from)?,
                    false => value,
                };
                self.adapt(value, from, to, result, at)
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
                if to.owned {
                    return unsupported("an array for a pointer that owns", at);
                }
                if matches!(array.kind, ExprKind::Str(_)) {
                    return unsupported("a string literal as a pointer: no rule yet", at);
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

    /// What an owner holds, moved out of it. A record or `Globals` stays in
    /// use, so what it holds is replaced by an empty value.
    fn moved(&self, pointer: &Expr, value: syn::Expr, from: Repr) -> R<syn::Expr> {
        let held = match pointer.kind {
            ExprKind::Member { .. } => true,
            ExprKind::Var(var) => self.program.vars[var].kind.is_global(),
            _ => false,
        };
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

    /// A pointer as an argument. Rust reborrows a `&mut` variable that is
    /// passed to a function, so it is passed by its name.
    pub(super) fn argument_pointer(&mut self, pointer: &Expr, to: Repr) -> R<syn::Expr> {
        if let ExprKind::Var(var) = pointer.kind
            && to.mutable
            && !to.owned
            && !to.nullable
            && self.repr(Slot::Var(var), pointer.at)? == to
        {
            return self.place(pointer, true);
        }
        self.pointer(pointer, to)
    }

    /// A cast between pointers keeps the element type, apart from `void`.
    fn same_element(&self, cast: &Expr, inner: &Expr) -> R<()> {
        if !inner.ty.is_pointer() {
            return unsupported("a pointer made from a number", cast.at);
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
            ExprKind::Var(_) | ExprKind::Member { .. } | ExprKind::Call(..) => {
                let repr = self.repr(self.slot_of(pointer)?, pointer.at)?;
                if repr.many {
                    return Ok(rust::index(self.slice(pointer, write)?, rust::int(0, "")));
                }
                let value = self.slot_value(pointer, write)?;
                let value = self.unwrapped(value, repr, write);
                Ok(if field {
                    value
                } else {
                    rust::unary("*", value)
                })
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
            ExprKind::Var(_) | ExprKind::Member { .. } | ExprKind::Call(..) => {
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
            ExprKind::Var(_) | ExprKind::Member { .. } | ExprKind::Call(..) => self
                .slot_of(pointer)
                .is_ok_and(|slot| !self.facts.pointer(slot).many),
            _ => false,
        }
    }

    /// An index or a count as `usize`.
    pub(super) fn index_value(&mut self, index: &Expr) -> R<syn::Expr> {
        if let Some(index @ 0..) = index.constant(self.program) {
            return Ok(rust::int(index, ""));
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
        let zero = self.zero(element, at)?;
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
        Ok(parse_quote!(vec![#zero; #count]))
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
