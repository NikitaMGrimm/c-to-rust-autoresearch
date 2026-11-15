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
    let text = format!("{:?}", value.abs());
    let negative: Option<syn::Token![-]> = value.is_sign_negative().then(Default::default);
    // Clippy refuses a literal that it takes for one of the constants of
    // `std::f64::consts` written out, in whatever notation. The C program
    // means the number it wrote, which is a whole number divided by a
    // power of ten: both are exact, so their quotient is the same `f64`
    // as the literal.
    if suffix != "f32"
        && near_constant(&text)
        && let Some((whole, fraction)) = text.split_once('.')
        && let Ok(digits @ ..EXACT) = format!("{whole}{fraction}").parse::<u64>()
        && fraction.len() <= 22
    {
        let digits = syn::LitFloat::new(&format!("{digits}.0{suffix}"), Span::call_site());
        let power = syn::LitFloat::new(&format!("1e{}", fraction.len()), Span::call_site());
        return parse_quote!(#negative #digits / #power);
    }
    let literal = syn::LitFloat::new(&format!("{text}{suffix}"), Span::call_site());
    parse_quote!(#negative #literal)
}

/// The whole numbers below this are `f64` values.
const EXACT: u64 = 1 << 53;

/// Whether Clippy's `approx_constant` may take the number for a constant
/// written out: its text is how a constant starts, or the constant rounded
/// to as many digits. Clippy asks for more digits for some constants and
/// knows more of them in some versions, and a number that it would have
/// let pass is as right as a quotient.
fn near_constant(text: &str) -> bool {
    use std::f64::consts::*;
    const KNOWN: [f64; 19] = [
        E,
        FRAC_1_PI,
        FRAC_1_SQRT_2,
        FRAC_2_PI,
        FRAC_2_SQRT_PI,
        FRAC_PI_2,
        FRAC_PI_3,
        FRAC_PI_4,
        FRAC_PI_6,
        FRAC_PI_8,
        LN_2,
        LN_10,
        LOG10_2,
        LOG10_E,
        LOG2_10,
        LOG2_E,
        PI,
        SQRT_2,
        TAU,
    ];
    // The golden ratio and Euler's constant, which `std` names only in
    // newer versions: as quotients, since Clippy refuses them here too.
    let later = [
        1_618_033_988_749_895.0 / 1e15,
        5_772_156_649_015_329.0 / 1e16,
    ];
    KNOWN.iter().chain(&later).any(|constant| {
        text.len() > 3
            && (constant.to_string().starts_with(text)
                || format!("{constant:.*}", text.len() - 2) == text)
    })
}

pub fn string(text: &str) -> Expr {
    let literal = syn::LitStr::new(text, Span::call_site());
    parse_quote!(#literal)
}

pub fn char(value: char) -> Expr {
    let literal = syn::LitChar::new(value, Span::call_site());
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

/// `field: value` in a record literal, or `field` alone where the value is
/// the variable of that name.
pub fn field_value(field: &str, value: Expr) -> syn::FieldValue {
    let member = syn::Member::Named(ident(field));
    let named =
        matches!(&value, Expr::Path(path) if path.qself.is_none() && path.path.is_ident(field));
    syn::FieldValue {
        attrs: Vec::new(),
        member,
        colon_token: (!named).then(Default::default),
        expr: value,
    }
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

/// `..end`, to take the start of a slice.
pub fn until(end: Expr) -> Expr {
    Expr::Range(syn::ExprRange {
        attrs: Vec::new(),
        start: None,
        limits: syn::RangeLimits::HalfOpen(Default::default()),
        end: Some(Box::new(end)),
    })
}

/// `start..end`.
pub fn range(start: Expr, end: Expr) -> Expr {
    Expr::Range(syn::ExprRange {
        attrs: Vec::new(),
        start: Some(Box::new(start)),
        limits: syn::RangeLimits::HalfOpen(Default::default()),
        end: Some(Box::new(end)),
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

/// A call of a function that a value holds, such as `p(x)`. A field
/// would read as a method, so it is called as `(s.op)(x)`.
pub fn call_value(function: Expr, args: impl IntoIterator<Item = Expr>) -> Expr {
    let function = match function {
        Expr::Field(_) => Expr::Paren(syn::ExprParen {
            attrs: Vec::new(),
            paren_token: Default::default(),
            expr: Box::new(function),
        }),
        function => function,
    };
    Expr::Call(syn::ExprCall {
        attrs: Vec::new(),
        func: Box::new(function),
        paren_token: Default::default(),
        args: Punctuated::from_iter(args),
    })
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
