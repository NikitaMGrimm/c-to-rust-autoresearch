//! Rules for types and for the values a type starts with.

use super::{Lower, R, unsupported};
use crate::c::{Expr, ExprKind, Type};
use crate::facts::Slot;
use crate::names::plain;
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
            Type::Array(element, Some(length)) => {
                let (element, length) = (self.ty(element, at)?, rust::int((*length).into(), ""));
                parse_quote!([#element; #length])
            }
            Type::Array(_, None) => {
                return unsupported("an array whose length is not a constant", at);
            }
            Type::Record(id) => {
                let record = &self.program.records[*id];
                if !record.own {
                    return unsupported(format!("the library's `{}`", record.name), at);
                }
                let name = rust::ident(self.names.record(*id));
                parse_quote!(#name)
            }
            Type::Pointer(_) => return unsupported("a pointer where no rule expects one", at),
            Type::Function { .. } => return unsupported("a function type", at),
        })
    }

    /// Whether the type's Rust values are copied by assignment, as C's are.
    pub(super) fn is_copy(&self, ty: &Type) -> bool {
        match ty {
            Type::Pointer(_) => false,
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
            _ => true,
        }
    }

    /// R2: the zero of a type, which a variable without an initialiser
    /// starts with. Reading such a variable is undefined in C, so any
    /// value is right, and Rust needs one.
    pub(super) fn zero(&self, ty: &Type, at: u32) -> R<syn::Expr> {
        Ok(match ty {
            Type::Bool => parse_quote!(false),
            Type::Char | Type::Int { .. } => rust::int(0, ""),
            Type::Float { .. } => rust::float(0.0, ""),
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
                parse_quote!(#name::default())
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
            (false, true, true) => parse_quote!(Vec::new()),
            (false, true, false) => parse_quote!(Box::default()),
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

    /// R5, R6: what an initialiser gives a place of type `ty`.
    pub(super) fn init(&mut self, ty: &Type, slot: Option<Slot>, init: &Expr) -> R<syn::Expr> {
        let program = self.program;
        match (ty, &init.kind) {
            (_, ExprKind::Zero) => self.zero_of(ty, slot, init.at),
            (Type::Pointer(_), _) => match slot {
                Some(slot) => {
                    let repr = self.repr(slot, init.at)?;
                    self.pointer(init, repr)
                }
                None => unsupported("a pointer in an array", init.at),
            },
            (Type::Record(id), ExprKind::InitList(items)) => {
                let name = rust::ident(self.names.record(*id));
                let mut values: Vec<syn::FieldValue> = Vec::new();
                for (index, field) in program.records[*id].fields.iter().enumerate() {
                    let slot = field.ty.is_pointer().then_some(Slot::Field(*id, index));
                    let value = match items.get(index) {
                        Some(item) => self.init(&field.ty, slot, item)?,
                        None => self.zero_of(&field.ty, slot, init.at)?,
                    };
                    let field = rust::ident(&plain(&field.name));
                    values.push(parse_quote!(#field: #value));
                }
                Ok(parse_quote!(#name { #(#values),* }))
            }
            (Type::Array(element, Some(length)), ExprKind::InitList(items)) => {
                let length = usize::try_from(*length).unwrap_or(usize::MAX);
                if items.is_empty() {
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
