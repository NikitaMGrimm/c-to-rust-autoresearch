//! Building the Rust syntax tree and printing it. The rules build
//! expressions with these functions, so that the printer adds the
//! parentheses an operand needs; `syn::parse_quote!` is for statements and
//! items, where every expression it takes stands alone between delimiters.

use proc_macro2::Span;
use syn::punctuated::Punctuated;
use syn::{Expr, parse_quote};

pub fn ident(name: &str) -> syn::Ident {
    syn::Ident::new(name, Span::call_site())
}

/// A variable or another single name.
pub fn name(name: &str) -> Expr {
    let ident = ident(name);
    parse_quote!(#ident)
}

/// An integer literal with a type suffix such as `u8`, or without.
pub fn int(value: i128, suffix: &str) -> Expr {
    let literal = syn::LitInt::new(
        &format!("{}{suffix}", value.unsigned_abs()),
        Span::call_site(),
    );
    let literal: Expr = parse_quote!(#literal);
    if value < 0 {
        unary("-", literal)
    } else {
        literal
    }
}

/// A float literal with a type suffix, or without.
pub fn float(value: f64, suffix: &str) -> Expr {
    if value.is_nan() {
        return parse_quote!(f64::NAN);
    }
    if value.is_infinite() {
        let infinity = parse_quote!(f64::INFINITY);
        return if value < 0.0 {
            unary("-", infinity)
        } else {
            infinity
        };
    }
    // `{:?}` prints the shortest text that reads back as the same number.
    let literal = syn::LitFloat::new(&format!("{:?}{suffix}", value.abs()), Span::call_site());
    let literal: Expr = parse_quote!(#literal);
    if value.is_sign_negative() {
        unary("-", literal)
    } else {
        literal
    }
}

pub fn string(text: &str) -> Expr {
    let literal = syn::LitStr::new(text, Span::call_site());
    parse_quote!(#literal)
}

pub fn byte(value: u8) -> Expr {
    let literal = syn::LitByte::new(value, Span::call_site());
    parse_quote!(#literal)
}

/// `left op right`, for an operator such as `+` or `+=`.
pub fn binary(left: Expr, op: &str, right: Expr) -> Expr {
    Expr::Binary(syn::ExprBinary {
        attrs: Vec::new(),
        left: Box::new(left),
        op: syn::parse_str(op).expect("a binary operator"),
        right: Box::new(right),
    })
}

/// `-operand`, `!operand` or `*operand`.
pub fn unary(op: &str, operand: Expr) -> Expr {
    Expr::Unary(syn::ExprUnary {
        attrs: Vec::new(),
        op: syn::parse_str(op).expect("a unary operator"),
        expr: Box::new(operand),
    })
}

pub fn assign(place: Expr, value: Expr) -> Expr {
    Expr::Assign(syn::ExprAssign {
        attrs: Vec::new(),
        left: Box::new(place),
        eq_token: Default::default(),
        right: Box::new(value),
    })
}

pub fn cast(value: Expr, ty: syn::Type) -> Expr {
    Expr::Cast(syn::ExprCast {
        attrs: Vec::new(),
        expr: Box::new(value),
        as_token: Default::default(),
        ty: Box::new(ty),
    })
}

pub fn field(base: Expr, field: &str) -> Expr {
    Expr::Field(syn::ExprField {
        attrs: Vec::new(),
        base: Box::new(base),
        dot_token: Default::default(),
        member: syn::Member::Named(ident(field)),
    })
}

pub fn index(base: Expr, index: Expr) -> Expr {
    Expr::Index(syn::ExprIndex {
        attrs: Vec::new(),
        expr: Box::new(base),
        bracket_token: Default::default(),
        index: Box::new(index),
    })
}

/// `start..`, to take the rest of a slice.
pub fn from(start: Expr) -> Expr {
    Expr::Range(syn::ExprRange {
        attrs: Vec::new(),
        start: Some(Box::new(start)),
        limits: syn::RangeLimits::HalfOpen(Default::default()),
        end: None,
    })
}

pub fn method(receiver: Expr, method: &str, args: impl IntoIterator<Item = Expr>) -> Expr {
    Expr::MethodCall(syn::ExprMethodCall {
        attrs: Vec::new(),
        receiver: Box::new(receiver),
        dot_token: Default::default(),
        method: ident(method),
        turbofish: None,
        paren_token: Default::default(),
        args: Punctuated::from_iter(args),
    })
}

/// A call of a function by its path, such as `Box::new`.
pub fn call(function: &str, args: impl IntoIterator<Item = Expr>) -> Expr {
    let function: syn::Path = syn::parse_str(function).expect("a path");
    let args = args.into_iter();
    parse_quote!(#function(#(#args),*))
}

/// `&value` or `&mut value`.
pub fn borrow(mutable: bool, value: Expr) -> Expr {
    Expr::Reference(syn::ExprReference {
        attrs: Vec::new(),
        and_token: Default::default(),
        mutability: mutable.then(Default::default),
        expr: Box::new(value),
    })
}

pub fn block(stmts: Vec<syn::Stmt>) -> syn::Block {
    parse_quote!({ #(#stmts)* })
}

/// The source text, with an empty line between items other than imports
/// and constants that follow one another.
pub fn print(file: &syn::File) -> String {
    let mut text = String::new();
    let mut before: Option<&syn::Item> = None;
    for item in &file.items {
        let short = |item: &syn::Item| {
            matches!(
                item,
                syn::Item::Use(_) | syn::Item::Const(_) | syn::Item::Static(_)
            )
        };
        if before.is_some_and(|before| !(short(before) && short(item))) {
            text.push('\n');
        }
        text.push_str(&prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: vec![item.clone()],
        }));
        before = Some(item);
    }
    text
}
