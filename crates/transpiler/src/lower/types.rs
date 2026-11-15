//! Rules for types and for the values a type starts with.

use super::{Lower, R, unsupported};
use crate::c::{Expr, ExprKind, Type};
use crate::facts::Slot;
use crate::rust;
use syn::parse_quote;

impl Lower<'_> {
    /// R2: the Rust type of a C type that holds a value. `char` is `u8`,
    /// since Rust keeps text as bytes; a pointer's type comes from its slot.
    pub(super) fn ty(&self, ty: &Type, at: u32) -> R<syn::Type> {
        Ok(match ty {
            Type::Void => parse_quote!(()),
            Type::Bool => parse_quote!(bool),
            Type::Char => parse_quote!(u8),
            Type::Int { bits, signed } => {
                let name = rust::ident(&format!("{}{bits}", if *signed { 'i' } else { 'u' }));
                parse_quote!(#name)
            }
            Type::Float { bits } => {
                let name = rust::ident(&format!("f{bits}"));
                parse_quote!(#name)
            }
            Type::Index => parse_quote!(usize),
            Type::Found => parse_quote!(Option<usize>),
            Type::Array(element, Some(length)) => {
                let (element, length) = (self.ty(element, at)?, rust::int((*length).into(), ""));
                parse_quote!([#element; #length])
            }
            Type::Array(_, None) => {
                return unsupported("an array whose length is not a constant", at);
            }
            Type::Record(id) => {
                let record = &self.program.records[*id];
                // R44: `div_t` is a record of the translation, written
                // where the program names it.
                if !record.own && record.fields.is_empty() {
                    return unsupported(format!("the library's `{}`", record.name), at);
                }
                if !record.own {
                    self.quotients.borrow_mut().insert(*id);
                }
                let name = rust::ident(self.names.record(*id));
                parse_quote!(#name)
            }
            // R42: an element of an array of pointers to functions.
            Type::Pointer(to) if matches!(**to, Type::Function { .. }) => self.fn_type(to, at)?,
            Type::Pointer(_) => return unsupported("a pointer where no rule expects one", at),
            Type::Function { .. } => return unsupported("a function type", at),
        })
    }

    /// Whether the type's Rust values are copied by assignment, as C's are.
    pub(super) fn is_copy(&self, ty: &Type) -> bool {
        match ty {
            Type::Pointer(_) => ty.function().is_some(),
            Type::Array(element, _) => self.is_copy(element),
            Type::Record(id) => self.program.records[*id]
                .fields
                .iter()
                .all(|field| self.is_copy(&field.ty)),
            _ => true,
        }
    }

    /// Whether `#[derive(Default)]` works for the type: the standard
    /// library has `Default` only for arrays of up to 32 elements.
    pub(super) fn derives_default(&self, ty: &Type) -> bool {
        match ty {
            Type::Array(element, length) => {
                length.is_some_and(|length| length <= 32) && self.derives_default(element)
            }
            Type::Record(id) => {
                self.program.records[*id]
                    .fields
                    .iter()
                    .all(|field| match &field.ty {
                        // A record has `Default` either way.
                        Type::Record(_) => true,
                        ty => self.derives_default(ty),
                    })
            }
            // A `fn` has no default.
            Type::Pointer(_) => ty.function().is_none(),
            _ => true,
        }
    }

    /// R2: the zero of a type, which a variable without an initialiser
    /// starts with. Reading such a variable is undefined in C, so any
    /// value is right, and Rust needs one.
    pub(super) fn zero(&self, ty: &Type, at: u32) -> R<syn::Expr> {
        Ok(match ty {
            Type::Bool => parse_quote!(false),
            Type::Char | Type::Int { .. } | Type::Index => rust::int(0, ""),
            Type::Float { .. } => rust::float(0.0, ""),
            Type::Found => parse_quote!(None),
            Type::Array(element, Some(length)) => {
                let (zero, length) = (self.zero(element, at)?, rust::int((*length).into(), ""));
                match self.is_copy(element) {
                    true => parse_quote!([#zero; #length]),
                    false => parse_quote!(std::array::from_fn(|_| #zero)),
                }
            }
            Type::Record(id) => {
                let name = rust::ident(self.names.record(*id));
                self.ty(ty, at)?;
                // A record has `Default` only where every field has a zero.
                for (index, field) in self.program.records[*id].fields.iter().enumerate() {
                    let slot = field.ty.is_pointer().then_some(Slot::Field(*id, index));
                    self.zero_of(&field.ty, slot, at)?;
                }
                parse_quote!(#name::default())
            }
            Type::Pointer(_) if ty.function().is_some() => {
                return unsupported("a pointer to a function that starts at NULL", at);
            }
            _ => return unsupported("a variable of this type without a value", at),
        })
    }

    /// The zero of a place that may hold a pointer, whose slot is `slot`.
    pub(super) fn zero_of(&self, ty: &Type, slot: Option<Slot>, at: u32) -> R<syn::Expr> {
        let (Type::Pointer(_), Some(slot)) = (ty, slot) else {
            return self.zero(ty, at);
        };
        let repr = self.repr(slot, at)?;
        Ok(match (repr.nullable, repr.owned, repr.many) {
            (true, ..) => parse_quote!(None),
            _ if ty.function().is_some() => {
                return unsupported(
                    format!(
                        "{} starts without a value and holds no NULL",
                        self.slot_name(slot)
                    ),
                    at,
                );
            }
            (false, true, true) => parse_quote!(Vec::new()),
            (false, true, false) => {
                if let Some(element) = ty.element() {
                    self.zero(element, at)?;
                }
                parse_quote!(Box::default())
            }
            (false, false, _) => {
                return unsupported(
                    format!(
                        "{} borrows and starts without a value",
                        self.slot_name(slot)
                    ),
                    at,
                );
            }
        })
    }

    /// R23: the width of the bit-field that the place is, if it is one
    /// that is narrower than its type.
    pub(super) fn bit_field(&self, place: &Expr) -> Option<u32> {
        let ExprKind::Member { record, field, .. } = &place.kind else {
            return None;
        };
        let field = &self.program.records[*record].fields[*field];
        let (bits, _) = field.ty.int()?;
        field.bits.filter(|&width| width < u32::from(bits))
    }

    /// R23: a value as a bit-field of `width` bits holds it: the low bits,
    /// with the sign of the highest of them for a signed type.
    pub(super) fn in_bits(&self, value: syn::Expr, ty: &Type, width: Option<u32>) -> syn::Expr {
        let (Some(width), Some((bits, signed))) = (width, ty.int()) else {
            return value;
        };
        // The bits of a number are known here, and `1 & 1` is an error to
        // the compiler's lints.
        if let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(number),
            ..
        }) = &value
            && number.suffix().is_empty()
            && let Ok(number) = number.base10_parse::<i128>()
        {
            let low = number & ((1 << width) - 1);
            return rust::int(
                match signed && low >> (width - 1) == 1 {
                    true => low - (1 << width),
                    false => low,
                },
                "",
            );
        }
        match signed {
            true => {
                let unused = rust::int(i128::from(bits) - i128::from(width), "");
                rust::binary(rust::binary(value, "<<", unused.clone()), ">>", unused)
            }
            false => rust::binary(value, "&", rust::int((1 << width) - 1, "")),
        }
    }

    /// Whether an initialiser gives the zero of its type and no pointer.
    fn is_zero(&self, init: &Expr) -> bool {
        match &init.kind {
            ExprKind::Zero => !init.ty.is_pointer(),
            ExprKind::Float(value) => *value == 0.0 && value.is_sign_positive(),
            ExprKind::InitList(items) => items.iter().all(|item| self.is_zero(item)),
            _ => init.ty.is_integer() && init.constant(self.program) == Some(0),
        }
    }

    /// R5, R6: what an initialiser gives a place of type `ty`.
    pub(super) fn init(&mut self, ty: &Type, slot: Option<Slot>, init: &Expr) -> R<syn::Expr> {
        let program = self.program;
        match (ty, &init.kind) {
            (_, ExprKind::Zero) => self.zero_of(ty, slot, init.at),
            (Type::Found, ExprKind::Null) => Ok(parse_quote!(None)),
            (Type::Found, _) => self.search(init.uncast()),
            (Type::Pointer(_), _) => match slot {
                Some(slot) => {
                    let repr = self.repr(slot, init.at)?;
                    self.pointer(init, repr)
                }
                None if ty.function().is_some() => self.function_value(init, false),
                None => unsupported("a pointer in an array", init.at),
            },
            (Type::Record(id), ExprKind::InitList(items)) => {
                let name = rust::ident(self.names.record(*id));
                let mut values: Vec<syn::FieldValue> = Vec::new();
                for (index, field) in program.records[*id].fields.iter().enumerate() {
                    let slot = field.ty.is_pointer().then_some(Slot::Field(*id, index));
                    let value = match items.get(index) {
                        Some(item) => {
                            let value = self.init(&field.ty, slot, item)?;
                            // Zero has no bits to cut off.
                            let width = field.bits.filter(|&width| {
                                !self.is_zero(item)
                                    && field
                                        .ty
                                        .int()
                                        .is_some_and(|(bits, _)| width < u32::from(bits))
                            });
                            self.in_bits(value, &field.ty, width)
                        }
                        None => self.zero_of(&field.ty, slot, init.at)?,
                    };
                    values.push(rust::field_value(self.names.field(*id, index), value));
                }
                Ok(parse_quote!(#name { #(#values),* }))
            }
            (Type::Array(element, Some(length)), ExprKind::InitList(items)) => {
                let length = usize::try_from(*length).unwrap_or(usize::MAX);
                // `{0}` and `{}` are the array of zeros, whatever its length.
                if items.iter().all(|item| self.is_zero(item)) {
                    return self.zero(ty, init.at);
                }
                let mut values = Vec::new();
                for item in items {
                    values.push(self.init(element, None, item)?);
                }
                // A few missing elements are written out; many are not.
                if length > values.len() + 8 || !self.is_copy(element) && length > values.len() {
                    return unsupported(
                        "an array whose initialiser leaves much of it out",
                        init.at,
                    );
                }
                while values.len() < length {
                    values.push(self.zero(element, init.at)?);
                }
                Ok(parse_quote!([#(#values),*]))
            }
            (Type::Array(element, Some(length)), ExprKind::Str(bytes))
                if **element == Type::Char =>
            {
                let length = usize::try_from(*length).unwrap_or(usize::MAX);
                if bytes.is_empty() {
                    return self.zero(ty, init.at);
                }
                if length > bytes.len() + 8 {
                    return unsupported("a string in a much longer array: no rule yet", init.at);
                }
                let mut bytes = bytes.clone();
                bytes.resize(length, 0);
                let literal = syn::LitByteStr::new(&bytes, proc_macro2::Span::call_site());
                Ok(parse_quote!(*#literal))
            }
            _ => self.typed(init),
        }
    }
}
