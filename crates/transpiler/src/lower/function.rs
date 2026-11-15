//! R42: pointers to functions, which are Rust's `fn` values. They point to
//! no memory, so of the facts about a pointer only `nullable` applies.

use super::{Lower, R, unsupported};
use crate::c::{Expr, ExprKind, Type};
use crate::facts::{Access, place_slot};
use crate::rust;
use syn::parse_quote;

impl Lower<'_> {
    /// The Rust type of a C function type: `fn(A, B) -> R`.
    pub(super) fn fn_type(&self, function: &Type, at: u32) -> R<syn::Type> {
        let Type::Function {
            ret,
            params,
            variadic,
        } = function
        else {
            return unsupported("a function type", at);
        };
        if *variadic {
            return unsupported(
                "a pointer to a function without a prototype or with variable arguments",
                at,
            );
        }
        // The functions that reach the pointer would have to agree on one
        // borrow.
        if params.iter().chain([&**ret]).any(Type::is_pointer) {
            return unsupported("a pointer to a function that takes a pointer", at);
        }
        let params = params
            .iter()
            .map(|param| self.ty(param, at))
            .collect::<R<Vec<_>>>()?;
        Ok(match &**ret {
            Type::Void => parse_quote!(fn(#(#params),*)),
            ret => {
                let ret = self.ty(ret, at)?;
                parse_quote!(fn(#(#params),*) -> #ret)
            }
        })
    }

    /// A pointer to a function as the value of a place that holds one, in
    /// `Option` if NULL may reach the place.
    pub(super) fn function_value(&mut self, value: &Expr, nullable: bool) -> R<syn::Expr> {
        let at = value.at;
        let wrap = |value: syn::Expr| match nullable {
            true => rust::call("Some", [value]),
            false => value,
        };
        let adapt = |value: syn::Expr, from: bool| match (from, nullable) {
            (false, true) => rust::call("Some", [value]),
            (true, false) => rust::method(value, "unwrap", []),
            _ => value,
        };
        match &value.kind {
            ExprKind::Function(name) => {
                let Some(id) = self.program.function(name) else {
                    return unsupported(
                        format!("the address of the library's `{name}`: no rule yet"),
                        at,
                    );
                };
                if name == "main" {
                    return unsupported("the address of `main`", at);
                }
                if self.facts.access(id) != Access::None {
                    return unsupported(
                        "a pointer to a function that uses globals: no rule yet",
                        at,
                    );
                }
                Ok(wrap(rust::name(self.names.function(id))))
            }
            ExprKind::Null => match nullable {
                true => Ok(parse_quote!(None)),
                false => unsupported("NULL for a pointer to a function that holds no NULL", at),
            },
            ExprKind::Cast(inner) if inner.ty == value.ty => self.function_value(inner, nullable),
            ExprKind::Cast(_) => unsupported("a cast of a pointer to a function", at),
            ExprKind::Cond(cond, then, other) => match self.decided(cond)? {
                Some(true) => self.function_value(then, nullable),
                Some(false) => self.function_value(other, nullable),
                None => {
                    let cond = self.cond(cond)?;
                    let then = self.function_value(then, nullable)?;
                    let other = self.function_value(other, nullable)?;
                    Ok(parse_quote!(if #cond { #then } else { #other }))
                }
            },
            ExprKind::Var(_) | ExprKind::Member { .. } => {
                let from = match place_slot(value) {
                    Some(slot) => self.repr(slot, at)?.nullable,
                    None => false,
                };
                let place = self.place(value, false)?;
                Ok(adapt(place, from))
            }
            // An array of pointers to functions holds no NULL.
            ExprKind::Index(..) => Ok(adapt(self.place(value, false)?, false)),
            ExprKind::Call(callee, args) => {
                let from = match callee
                    .function()
                    .and_then(|name| self.program.function(name))
                {
                    Some(id) => self.repr(crate::facts::Slot::Return(id), at)?.nullable,
                    None => false,
                };
                let call = self.call(value, callee, args)?;
                Ok(adapt(call, from))
            }
            _ => unsupported("a pointer to a function with no rule", at),
        }
    }

    /// A call through a pointer to a function: `p(x)`, `(s.op)(x)`.
    pub(super) fn call_through(
        &mut self,
        call: &Expr,
        callee: &Expr,
        args: &[Expr],
    ) -> R<syn::Expr> {
        let Some(function @ Type::Function { params, .. }) = callee.ty.function() else {
            return unsupported("a call through a pointer", call.at);
        };
        self.fn_type(function, call.at)?;
        if params.len() != args.len() {
            return unsupported("a call through a pointer with other arguments", call.at);
        }
        // C's behaviour is undefined where the pointer is NULL.
        let target = self.function_value(callee, false)?;
        let values = args
            .iter()
            .map(|arg| self.typed(arg))
            .collect::<R<Vec<_>>>()?;
        Ok(rust::call_value(target, values))
    }
}
