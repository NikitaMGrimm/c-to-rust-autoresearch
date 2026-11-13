//! Rules for the C library's functions. A function is translated to the
//! standard library where that behaves the same, and to a `Helper`
//! otherwise.

use super::pointer::Repr;
use super::{Helper, Lower, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Type};
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
            ("printf", [format, values @ ..]) => self.printf(format, values, Stream::Out, at),
            // R26: the same on a stream that Rust has a macro for.
            ("fprintf", [stream, format, values @ ..]) => {
                let stream = Stream::of(stream, name, at)?;
                self.printf(format, values, stream, at)
            }
            ("puts", [text]) => self.puts(text, Stream::Out, true),
            ("fputs", [text, stream]) => {
                let stream = Stream::of(stream, name, at)?;
                self.puts(text, stream, false)
            }
            ("strlen", [text]) => {
                let text = match self.reads_bytes(text) {
                    true => self.until_nul(text)?,
                    false => self.text(text, false)?,
                };
                Ok(rust::cast(rust::method(text, "len", []), parse_quote!(u64)))
            }
            ("fgets", [buffer, size, stream]) => self.read_line(buffer, size, stream, at),
            ("memset", [target, value, size]) => self.memset(target, value, size, at),
            // R22: the text of `source`, cut short or filled up with NUL
            // bytes to the size.
            ("strncpy", [target, source, size]) => {
                let target = rust::borrow(true, self.first(target, Err(size))?);
                let source = self.pointer(source, Repr::BYTES)?;
                self.helper(Helper::CopyText);
                Ok(rust::call("copy_text", [target, source]))
            }
            ("putchar", [value]) => {
                let value = self.character(value)?;
                Ok(parse_quote!(print!("{}", #value)))
            }
            ("fputc" | "putc", [value, stream]) => {
                let print = Stream::of(stream, name, at)?.print();
                let value = self.character(value)?;
                Ok(parse_quote!(#print!("{}", #value)))
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
            // R21 covers an end pointer where `normalise::ends` made the
            // call a statement of its own.
            ("strtol" | "strtoll", [_, end, _]) if !matches!(end.uncast().kind, ExprKind::Null) => {
                unsupported(
                    "an end pointer other than a local `char *` that one call sets before each read: no rule yet",
                    at,
                )
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
    pub(super) fn text(&mut self, text: &Expr, borrowed: bool) -> R<syn::Expr> {
        if self.is_argument(text) {
            let argument = self.argument(text)?;
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

    /// Whether the text is in bytes that end with a NUL, rather than in a
    /// `&str`.
    fn reads_bytes(&self, text: &Expr) -> bool {
        literal_text(text).is_none() && !self.is_argument(text) && !self.is_text(text)
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
        if let (Some(size), Some(room)) = (size.constant(self.program), self.room(buffer))
            && size <= room
        {
            let target = rust::borrow(true, self.first(buffer, Ok(size))?);
            self.helper(Helper::ReadLine);
            return Ok(rust::call("read_line", [target]));
        }
        // `fgets` stores no further than its text reaches, so a size
        // beyond the buffer is no error in C until the text is as long,
        // and a slice of that size is one at once.
        let bytes = Repr {
            mutable: true,
            ..Repr::BYTES
        };
        let target = self.pointer(buffer.uncast(), bytes)?;
        let size = self.expr(size)?;
        self.helper(Helper::ReadLineWithin);
        Ok(rust::call("read_line_within", [target, size]))
    }

    /// How many elements the pointer is known to have room for: those of
    /// an array, less a constant offset into it.
    fn room(&self, pointer: &Expr) -> Option<i128> {
        match &pointer.kind {
            ExprKind::Cast(inner) => self.room(inner),
            ExprKind::Decay(array) => match &array.ty {
                Type::Array(_, Some(length)) => Some(i128::from(*length)),
                _ => None,
            },
            ExprKind::Binary(BinOp::Add, left, right) => {
                let (base, offset) = match left.ty.is_pointer() {
                    true => (left, right),
                    false => (right, left),
                };
                let offset = offset
                    .constant(self.program)
                    .filter(|offset| *offset >= 0)?;
                Some(self.room(base)? - offset)
            }
            _ => None,
        }
    }

    /// The first `count` elements that a pointer points to, as a place to
    /// store to: the whole array when that is what the count says, and
    /// `array[..count]` otherwise. The count is a number or an expression.
    fn first(&mut self, pointer: &Expr, count: Result<i128, &Expr>) -> R<syn::Expr> {
        let count = count.or_else(|count| count.constant(self.program).ok_or(count));
        let whole = match &pointer.uncast().kind {
            ExprKind::Decay(array) => match &array.ty {
                Type::Array(_, Some(length)) => count.ok() == Some(i128::from(*length)),
                _ => false,
            },
            _ => false,
        };
        let slice = self.slice(pointer.uncast(), true)?;
        Ok(match (whole, count) {
            (true, _) => slice,
            (false, Ok(count @ 0..)) => rust::index(slice, rust::until(rust::int(count, ""))),
            (false, Ok(_)) => return unsupported("a negative size", pointer.at),
            (false, Err(count)) => rust::index(slice, rust::until(self.index_value(count)?)),
        })
    }

    /// R22: `memset` as `fill`, of bytes with a byte and of numbers with
    /// zero, or as the zero value of one variable.
    fn memset(&mut self, target: &Expr, value: &Expr, size: &Expr, at: u32) -> R<syn::Expr> {
        let program = self.program;
        let target = target.uncast();
        let Some(element) = target.ty.element() else {
            return unsupported("`memset` of something that is no pointer", at);
        };
        let byte = Type::Int {
            bits: 8,
            signed: false,
        };
        if *element == Type::Char || *element == byte {
            let value = match value.constant(program) {
                Some(value @ 0..=127) => rust::byte(value as u8),
                _ => rust::cast(self.expr(value)?, parse_quote!(u8)),
            };
            return Ok(rust::method(
                self.first(target, Err(size))?,
                "fill",
                [value],
            ));
        }
        // Zero bytes are the zero of every number, and nothing else is the
        // same for every type.
        let (Some(0), Some(bytes), Some((each, _))) = (
            value.constant(program),
            size.constant(program),
            element.layout(program),
        ) else {
            return unsupported("this `memset`: no rule yet", at);
        };
        let each = i128::from(each);
        if !self.is_copy(element) || bytes % each != 0 {
            return unsupported("this `memset`: no rule yet", at);
        }
        let zero = self.zero(element, at)?;
        if let (ExprKind::AddrOf(place), true) = (&target.kind, bytes == each) {
            return Ok(rust::assign(self.place(place, true)?, zero));
        }
        Ok(rust::method(
            self.first(target, Ok(bytes / each))?,
            "fill",
            [zero],
        ))
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

    /// R4: `puts` as `println!`, and R26: `fputs` as `print!` or `eprint!`.
    fn puts(&mut self, text: &Expr, stream: Stream, line_end: bool) -> R<syn::Expr> {
        let print = match line_end {
            true => stream.println(),
            false => stream.print(),
        };
        Ok(match literal_text(text) {
            Some(text) => {
                let text = text.replace('{', "{{").replace('}', "}}");
                let (print, text) = match text.strip_suffix('\n') {
                    Some(line) if !line_end => (stream.println(), line),
                    _ => (print, text.as_str()),
                };
                let text = rust::string(text);
                parse_quote!(#print!(#text))
            }
            None => {
                let text = self.text(text, false)?;
                parse_quote!(#print!("{}", #text))
            }
        })
    }

    /// R4: `printf` with a literal format as `print!` or `println!`, and
    /// R26 as `eprint!` or `eprintln!` on `stderr`.
    fn printf(&mut self, format: &Expr, values: &[Expr], stream: Stream, at: u32) -> R<syn::Expr> {
        let (text, arguments) = self.format(format, values, at)?;
        let (print, println) = (stream.print(), stream.println());
        Ok(match text.strip_suffix('\n') {
            Some(line) => {
                let line = rust::string(line);
                parse_quote!(#println!(#line #(, #arguments)*))
            }
            None => {
                let text = rust::string(&text);
                parse_quote!(#print!(#text #(, #arguments)*))
            }
        })
    }

    /// A literal `printf` format converted to Rust's, with the values it
    /// prints.
    fn format(&mut self, format: &Expr, values: &[Expr], at: u32) -> R<(String, Vec<syn::Expr>)> {
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
        Ok((text, arguments))
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
            (Some(_), false, false) if matches!(letter, b's' | b'c' | b'e' | b'g') => {
                spec.push('>')
            }
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
            // `%F` prints an infinity in capitals, which Rust does not.
            b'f' => {
                spec.push_str(&format!(".{}", precision.unwrap_or(6)));
                self.expr(value)?
            }
            // R19: the helpers give the text that C prints, which Rust
            // then only pads.
            b'e' | b'g' => {
                if zero || plus {
                    return unsupported("a `printf` flag for `%e` or `%g`: no rule yet", at);
                }
                let helper = match letter {
                    b'e' => Helper::FormatE,
                    _ => Helper::FormatG,
                };
                self.helper(helper);
                let precision = rust::int(precision.unwrap_or(6) as i128, "");
                let value = self.expr(value)?;
                match helper {
                    Helper::FormatE => rust::call("format_e", [value, precision]),
                    _ => rust::call("format_g", [value, precision]),
                }
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
                    None if width.is_some() && !self.is_argument(value) => {
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

/// A stream that Rust prints to with a macro of its own.
#[derive(Clone, Copy)]
enum Stream {
    Out,
    Err,
}

impl Stream {
    /// `stdout` or `stderr` as the stream argument of `name`.
    fn of(stream: &Expr, name: &str, at: u32) -> R<Stream> {
        match &stream.uncast().kind {
            ExprKind::Extern(stream) if stream == "stdout" => Ok(Stream::Out),
            ExprKind::Extern(stream) if stream == "stderr" => Ok(Stream::Err),
            _ => unsupported(
                format!("`{name}` on a file other than `stdout` or `stderr`: no rule yet"),
                at,
            ),
        }
    }

    fn print(self) -> syn::Ident {
        rust::ident(match self {
            Stream::Out => "print",
            Stream::Err => "eprint",
        })
    }

    fn println(self) -> syn::Ident {
        rust::ident(match self {
            Stream::Out => "println",
            Stream::Err => "eprintln",
        })
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
