//! Rules for the C library's functions. A function is translated to the
//! standard library where that behaves the same, and to a `Helper`
//! otherwise.

use super::pointer::Repr;
use super::{Helper, Lower, R, unsupported};
use crate::c::{Expr, ExprKind, Type};
use crate::rust;
use syn::parse_quote;

/// One conversion of a `printf` format, such as `%-5.2f`.
struct Conversion {
    left: bool,
    zero: bool,
    plus: bool,
    width: Option<usize>,
    precision: Option<usize>,
    /// The length modifier: `hh`, `h`, `l`, `ll` or `z`, or none.
    length: String,
    letter: u8,
}

impl Lower<'_> {
    pub(super) fn library(&mut self, call: &Expr, name: &str, args: &[Expr]) -> R<syn::Expr> {
        let at = call.at;
        match (name, args) {
            ("printf", [format, values @ ..]) => self.printf(format, values, at),
            ("puts", [text]) => match literal_text(text) {
                Some(text) => {
                    let line = rust::string(&text.replace('{', "{{").replace('}', "}}"));
                    Ok(parse_quote!(println!(#line)))
                }
                None => {
                    let text = self.text(text, false)?;
                    Ok(parse_quote!(println!("{}", #text)))
                }
            },
            ("strlen", [text]) => {
                let text = match self.reads_bytes(text) {
                    true => self.until_nul(text)?,
                    false => self.text(text, false)?,
                };
                Ok(rust::cast(rust::method(text, "len", []), parse_quote!(u64)))
            }
            ("fgets", [buffer, size, stream]) => self.read_line(buffer, size, stream, at),
            ("putchar", [value]) => {
                let value = self.character(value)?;
                Ok(parse_quote!(print!("{}", #value)))
            }
            ("free", [pointer]) => self.free(pointer),
            ("exit", [code]) => Ok(rust::call("std::process::exit", [self.typed(code)?])),
            ("abs" | "labs" | "llabs", [value]) => Ok(rust::method(self.expr(value)?, "abs", [])),
            ("sqrt" | "fabs" | "floor" | "ceil", [value]) => {
                let method = match name {
                    "fabs" => "abs",
                    name => name,
                };
                Ok(rust::method(self.expr(value)?, method, []))
            }
            ("pow", [base, exponent]) => {
                let (base, exponent) = (self.expr(base)?, self.typed(exponent)?);
                Ok(rust::method(base, "powf", [exponent]))
            }
            // R4: a number from a program argument.
            ("strtol" | "strtoll", [text, end, base])
                if matches!(end.uncast().kind, ExprKind::Null)
                    && base.constant(self.program) == Some(10) =>
            {
                self.parse_int(text)
            }
            ("strtoul" | "strtoull", [text, end, base])
                if matches!(end.uncast().kind, ExprKind::Null)
                    && base.constant(self.program) == Some(10) =>
            {
                let text = self.text(text, true)?;
                self.helper(Helper::ParseUint);
                Ok(rust::call("parse_uint", [text]))
            }
            ("strtod" | "strtof", [text, end]) if matches!(end.uncast().kind, ExprKind::Null) => {
                self.parse_float(text, &call.ty, at)
            }
            ("atof", [text]) => self.parse_float(text, &call.ty, at),
            ("atoi", [text]) => Ok(rust::cast(self.parse_int(text)?, parse_quote!(i32))),
            ("atol" | "atoll", [text]) => self.parse_int(text),
            _ => unsupported(
                format!("this call of the library's `{name}`: no rule yet"),
                at,
            ),
        }
    }

    /// `parse_int` of text.
    fn parse_int(&mut self, text: &Expr) -> R<syn::Expr> {
        let text = self.text(text, true)?;
        self.helper(Helper::ParseInt);
        Ok(rust::call("parse_int", [text]))
    }

    /// R18: `parse_float` of text, as an `f32` or an `f64`.
    fn parse_float(&mut self, text: &Expr, ty: &Type, at: u32) -> R<syn::Expr> {
        let text = self.text(text, true)?;
        let ty = self.ty(ty, at)?;
        self.helper(Helper::ParseFloat);
        Ok(parse_quote!(parse_float::<#ty>(#text)))
    }

    /// R13: text that a library function reads as a whole: a string
    /// literal, a pointer that only holds text, or `argv[i]` as `args[i]`.
    /// `borrowed` asks for a `&str`, where `args[i]` is a `String`.
    fn text(&mut self, text: &Expr, borrowed: bool) -> R<syn::Expr> {
        if let Some(index) = self.argument(text) {
            let index = self.index_value(index)?;
            let argument = rust::index(rust::name("args"), index);
            return Ok(match borrowed {
                true => rust::borrow(false, argument),
                false => argument,
            });
        }
        if !self.reads_bytes(text) {
            return self.pointer(text, Repr::TEXT);
        }
        let bytes = self.pointer(text, Repr::BYTES)?;
        self.helper(Helper::TextOf);
        let text = rust::call("text_of", [bytes]);
        Ok(match borrowed {
            true => rust::borrow(false, text),
            false => text,
        })
    }

    /// The index of `argv[index]`, if the expression is one of `main`'s
    /// arguments.
    fn argument<'e>(&self, text: &'e Expr) -> Option<&'e Expr> {
        match &text.uncast().kind {
            ExprKind::Index(base, index) if matches!(base.kind, ExprKind::Var(var) if Some(var) == self.argv) => {
                Some(index)
            }
            _ => None,
        }
    }

    /// Whether the text is in bytes that end with a NUL, rather than in a
    /// `&str`.
    fn reads_bytes(&self, text: &Expr) -> bool {
        literal_text(text).is_none() && self.argument(text).is_none() && !self.is_text(text)
    }

    /// R14: the bytes of the text that a pointer to bytes points to.
    fn until_nul(&mut self, text: &Expr) -> R<syn::Expr> {
        let bytes = self.pointer(text, Repr::BYTES)?;
        self.helper(Helper::UntilNul);
        Ok(rust::call("until_nul", [bytes]))
    }

    /// R15: `fgets` on `stdin` as `read_line`, which is true where `fgets`
    /// returns the buffer and false where it returns NULL.
    pub(super) fn read_line(
        &mut self,
        buffer: &Expr,
        size: &Expr,
        stream: &Expr,
        at: u32,
    ) -> R<syn::Expr> {
        if !matches!(&stream.uncast().kind, ExprKind::Extern(name) if name == "stdin") {
            return unsupported("`fgets` on a file other than `stdin`: no rule yet", at);
        }
        // The whole array, when that is what the size says.
        let whole = match &buffer.uncast().kind {
            ExprKind::Decay(array) => match &array.ty {
                Type::Array(_, Some(length)) => {
                    size.constant(self.program) == Some(i128::from(*length))
                }
                _ => false,
            },
            _ => false,
        };
        let target = match whole {
            true => self.pointer(
                buffer,
                Repr {
                    mutable: true,
                    ..Repr::BYTES
                },
            )?,
            false => {
                let size = rust::until(self.index_value(size)?);
                rust::borrow(true, rust::index(self.slice(buffer, true)?, size))
            }
        };
        self.helper(Helper::ReadLine);
        Ok(rust::call("read_line", [target]))
    }

    /// A value that C prints with `%c`, as a `char`.
    fn character(&mut self, value: &Expr) -> R<syn::Expr> {
        // A `char` that C widened to `int` is the byte itself.
        let byte = match &value.kind {
            ExprKind::Cast(inner) if inner.ty == Type::Char => self.expr(inner)?,
            _ => rust::cast(self.expr(value)?, parse_quote!(u8)),
        };
        Ok(rust::cast(byte, parse_quote!(char)))
    }

    /// R4: `printf` with a literal format as `print!` or `println!`, the
    /// format converted here.
    fn printf(&mut self, format: &Expr, values: &[Expr], at: u32) -> R<syn::Expr> {
        let ExprKind::Decay(literal) = &format.uncast().kind else {
            return unsupported("`printf` with a format that is not a literal", at);
        };
        let ExprKind::Str(format) = &literal.kind else {
            return unsupported("`printf` with a format that is not a literal", at);
        };
        // The format ends at its first NUL, as in C.
        let format = format.split(|&byte| byte == 0).next().unwrap_or_default();
        let Ok(format) = std::str::from_utf8(format) else {
            return unsupported("a `printf` format that is not UTF-8", at);
        };
        let mut text = String::new();
        let mut arguments = Vec::new();
        let mut values = values.iter();
        let mut rest = format;
        while let Some(percent) = rest.find('%') {
            text.push_str(&rest[..percent].replace('{', "{{").replace('}', "}}"));
            rest = &rest[percent + 1..];
            if let Some(after) = rest.strip_prefix('%') {
                text.push('%');
                rest = after;
                continue;
            }
            let Some((conversion, after)) = conversion(rest) else {
                return unsupported("a `printf` conversion with no rule", at);
            };
            rest = after;
            let Some(value) = values.next() else {
                return unsupported("a `printf` conversion without a value", at);
            };
            let (spec, argument) = self.converted(&conversion, value, at)?;
            text.push_str(&spec);
            arguments.push(argument);
        }
        text.push_str(&rest.replace('{', "{{").replace('}', "}}"));
        if values.next().is_some() {
            return unsupported("a `printf` value without a conversion", at);
        }
        Ok(match text.strip_suffix('\n') {
            Some(line) => {
                let line = rust::string(line);
                parse_quote!(println!(#line #(, #arguments)*))
            }
            None => {
                let text = rust::string(&text);
                parse_quote!(print!(#text #(, #arguments)*))
            }
        })
    }

    /// Rust's format for one conversion, and the value it prints.
    fn converted(
        &mut self,
        conversion: &Conversion,
        value: &Expr,
        at: u32,
    ) -> R<(String, syn::Expr)> {
        let Conversion {
            left,
            zero,
            plus,
            width,
            precision,
            ..
        } = *conversion;
        let letter = conversion.letter;
        let mut spec = String::new();
        // C pads on the left unless told otherwise; Rust does so only for
        // numbers.
        match (width, left, zero) {
            (Some(_), true, _) => spec.push('<'),
            (Some(_), false, false) if letter == b's' || letter == b'c' => spec.push('>'),
            _ => {}
        }
        if plus {
            spec.push('+');
        }
        if zero && !left {
            spec.push('0');
        }
        if let Some(width) = width {
            spec.push_str(&width.to_string());
        }
        let argument = match letter {
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' => {
                if precision.is_some() {
                    return unsupported("a `printf` precision for an integer: no rule yet", at);
                }
                // The conversion says how C reads the value, whatever the
                // value's own type is.
                let bits = match conversion.length.as_str() {
                    "hh" => 8,
                    "h" => 16,
                    "" => 32,
                    _ => 64,
                };
                let read = Type::Int {
                    bits,
                    signed: matches!(letter, b'd' | b'i'),
                };
                match letter {
                    b'x' | b'X' | b'o' => spec.push(char::from(letter)),
                    _ => {}
                }
                match value.ty == read {
                    true => self.expr(value)?,
                    false => rust::cast(self.expr(value)?, self.ty(&read, at)?),
                }
            }
            b'f' | b'F' => {
                spec.push_str(&format!(".{}", precision.unwrap_or(6)));
                self.expr(value)?
            }
            b'c' => self.character(value)?,
            b's' => {
                if precision.is_some() {
                    return unsupported("a `printf` precision for text: no rule yet", at);
                }
                // C pads to a width in bytes and Rust in characters.
                match literal_text(value) {
                    Some(text) if width.is_some() && !text.is_ascii() => {
                        return unsupported("a `printf` width for text that is not ASCII", at);
                    }
                    Some(text) => rust::string(text),
                    // What the pointer holds may not be ASCII either.
                    None if width.is_some() && self.argument(value).is_none() => {
                        return unsupported("a `printf` width for text in a variable", at);
                    }
                    None => self.text(value, false)?,
                }
            }
            other => {
                return unsupported(
                    format!(
                        "the `printf` conversion `%{}`: no rule yet",
                        char::from(other)
                    ),
                    at,
                );
            }
        };
        Ok((
            match spec.is_empty() {
                true => "{}".to_owned(),
                false => format!("{{:{spec}}}"),
            },
            argument,
        ))
    }
}

/// The text of a string literal, if the expression is one that holds UTF-8
/// without a NUL.
fn literal_text(text: &Expr) -> Option<&str> {
    let ExprKind::Decay(literal) = &text.uncast().kind else {
        return None;
    };
    let ExprKind::Str(bytes) = &literal.kind else {
        return None;
    };
    let text = std::str::from_utf8(bytes).ok()?;
    (!text.contains('\0')).then_some(text)
}

/// Reads one conversion after its `%`, and returns it with what follows.
fn conversion(format: &str) -> Option<(Conversion, &str)> {
    let mut rest = format;
    let mut conversion = Conversion {
        left: false,
        zero: false,
        plus: false,
        width: None,
        precision: None,
        length: String::new(),
        letter: 0,
    };
    loop {
        match rest.as_bytes().first()? {
            b'-' => conversion.left = true,
            b'0' => conversion.zero = true,
            b'+' => conversion.plus = true,
            // ` ` and `#` have no Rust format that prints the same.
            _ => break,
        }
        rest = &rest[1..];
    }
    let number = |rest: &mut &str| {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let number = rest[..digits].parse().ok();
        *rest = &rest[digits..];
        number
    };
    conversion.width = number(&mut rest);
    if let Some(after) = rest.strip_prefix('.') {
        rest = after;
        conversion.precision = Some(number(&mut rest).unwrap_or(0));
    }
    let length = rest
        .find(|c: char| !matches!(c, 'h' | 'l' | 'z' | 'j' | 't'))
        .unwrap_or(rest.len());
    conversion.length = rest[..length].to_owned();
    rest = &rest[length..];
    conversion.letter = *rest.as_bytes().first()?;
    Some((conversion, &rest[1..]))
}
