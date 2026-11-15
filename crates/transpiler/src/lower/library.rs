//! Rules for the C library's functions. A function is translated to the
//! standard library where that behaves the same, and to a `Helper`
//! otherwise.

use super::expr::{round_trip, symbol};
use super::pointer::Repr;
use super::{Helper, Lower, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Type};
use crate::rust;
use quote::ToTokens;
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
            ("sprintf", [target, format, values @ ..]) => {
                self.sprintf(target, None, format, values, at)
            }
            ("snprintf", [target, size, format, values @ ..]) => {
                self.sprintf(target, Some(size), format, values, at)
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
            ("memcpy" | "memmove", [target, source, size]) => self.memcpy(target, source, size, at),
            // R22: the text of `source`, cut short or filled up with NUL
            // bytes to the size.
            ("strncpy", [target, source, size]) => {
                let target = rust::borrow(true, self.first(target, Err(size))?);
                let source = self.pointer(source, Repr::BYTES)?;
                self.helper(Helper::CopyText);
                Ok(rust::call("copy_text", [target, source]))
            }
            // R28: the text of `source` with its NUL, stored at the start
            // or at the end of the text in `target`.
            ("strcpy" | "strcat" | "strncat", [target, source, ..])
                if let (Some((array, _)), Some((other, _))) = (split(target), split(source))
                    && same_place(array, other) =>
            {
                // Rust borrows the array once to store and once to read.
                unsupported(format!("`{name}` within one array: no rule yet"), at)
            }
            ("strcpy" | "strcat", [target, source]) => {
                let target = self.pointer(target, Repr::BYTES_WRITTEN)?;
                let source = self.pointer(source, Repr::BYTES)?;
                let (helper, function) = match name {
                    "strcpy" => (Helper::CopyString, "copy_string"),
                    _ => (Helper::AppendString, "append_string"),
                };
                self.helper(helper);
                Ok(rust::call(function, [target, source]))
            }
            ("strncat", [target, source, count]) => {
                let target = self.pointer(target, Repr::BYTES_WRITTEN)?;
                let source = self.pointer(source, Repr::BYTES)?;
                let count = self.index_value(count)?;
                self.helper(Helper::AppendAtMost);
                Ok(rust::call("append_at_most", [target, source, count]))
            }
            // R30: C fixes only the sign of the result, and C libraries
            // differ in the number.
            ("strcmp" | "strncmp" | "memcmp", _) => unsupported(
                format!("the value of `{name}` other than its sign: no rule yet"),
                at,
            ),
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
            // R46: the numbers of `rand` are those of a `Random` that the
            // one function that calls it holds.
            ("rand", []) => {
                self.random(name, at)?;
                Ok(parse_quote!(random.next()))
            }
            ("srand", [seed]) => {
                self.random(name, at)?;
                let seed = self.typed(seed)?;
                Ok(parse_quote!(random = Random::new(#seed)))
            }
            ("exit", [code]) => Ok(rust::call("std::process::exit", [self.typed(code)?])),
            ("abs" | "labs" | "llabs", [value]) => Ok(rust::method(self.expr(value)?, "abs", [])),
            ("sqrt" | "fabs" | "floor" | "ceil", [value]) => {
                let method = match name {
                    "fabs" => "abs",
                    name => name,
                };
                Ok(rust::method(self.receiver(value)?, method, []))
            }
            // R38 writes these to test what a function of `math.h` reports.
            ("__builtin_isnan" | "__builtin_isinf" | "__builtin_isfinite", [value])
                if call.ty == Type::Bool =>
            {
                let method = match name {
                    "__builtin_isnan" => "is_nan",
                    "__builtin_isinf" => "is_infinite",
                    _ => "is_finite",
                };
                Ok(rust::method(self.expr(value)?, method, []))
            }
            ("pow", [base, exponent]) => {
                let (base, exponent) = (self.receiver(base)?, self.typed(exponent)?);
                Ok(rust::method(base, "powf", [exponent]))
            }
            // R21 covers an end pointer where `normalise::ends` made the
            // call a statement of its own.
            ("strtol" | "strtoll" | "strtoul" | "strtoull", [_, end, _])
            | ("strtod" | "strtof", [_, end])
                if !matches!(end.uncast().kind, ExprKind::Null) =>
            {
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
            // R44: the quotient and the remainder as a record. C divides
            // towards zero, as Rust does.
            ("div" | "ldiv" | "lldiv", [number, by]) => {
                let Type::Record(record) = call.ty else {
                    return unsupported(
                        format!("this call of the library's `{name}`: no rule yet"),
                        at,
                    );
                };
                if !number.is_pure() || !by.is_pure() {
                    return unsupported(
                        format!("`{name}` of a value with a side effect: no rule yet"),
                        at,
                    );
                }
                let ty = self.ty(&call.ty, at)?;
                let (number, by) = (self.expr(number)?, self.expr(by)?);
                let quot = rust::ident(self.names.field(record, 0));
                let rem = rust::ident(self.names.field(record, 1));
                let quotient = rust::binary(number.clone(), "/", by.clone());
                let remainder = rust::binary(number, "%", by);
                Ok(parse_quote!(#ty { #quot: #quotient, #rem: #remainder }))
            }
            ("atoi", [text]) => Ok(rust::cast(self.parse_int(text)?, parse_quote!(i32))),
            ("atol" | "atoll", [text]) => self.parse_int(text),
            _ => unsupported(
                format!("this call of the library's `{name}`: no rule yet"),
                at,
            ),
        }
    }

    /// A number whose method is called: a constant needs its type, which
    /// Rust does not infer for `10.0.powf(x)`.
    fn receiver(&mut self, value: &Expr) -> R<syn::Expr> {
        let suffix = match value.ty {
            Type::Float { bits: 32 } => "f32",
            _ => "f64",
        };
        match value.uncast().kind {
            ExprKind::Float(number) if value.uncast().ty == value.ty => {
                Ok(rust::float(number, suffix))
            }
            ExprKind::Int(number) if value.ty.int().is_none() => {
                Ok(rust::float(number as f64, suffix))
            }
            _ => self.expr(value),
        }
    }

    /// `parse_int` of text.
    fn parse_int(&mut self, text: &Expr) -> R<syn::Expr> {
        let text = self.text(text, true)?;
        self.helper(Helper::ParseInt);
        Ok(rust::call("parse_int", [text]))
    }

    /// R46: notes that the function holds the program's `Random`, or
    /// refuses a call of `rand` or `srand` where another function calls
    /// them too: the numbers of the two would have to be one sequence.
    fn random(&mut self, name: &str, at: u32) -> R<()> {
        if self.program.callers(&["rand", "srand"]).len() != 1 {
            return unsupported(
                format!("`{name}` in more than one function: no rule yet"),
                at,
            );
        }
        if self.program.callers(&["rand"]).is_empty() {
            return unsupported("`srand` in a program without `rand`", at);
        }
        self.helper(Helper::Random);
        Ok(())
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
    pub(super) fn reads_bytes(&self, text: &Expr) -> bool {
        literal_text(text).is_none() && !self.is_argument(text) && !self.is_text(text)
    }

    /// R14: the bytes of the text that a pointer to bytes points to.
    fn until_nul(&mut self, text: &Expr) -> R<syn::Expr> {
        let bytes = self.pointer(text, Repr::BYTES)?;
        self.helper(Helper::UntilNul);
        Ok(rust::call("until_nul", [bytes]))
    }

    /// R30: `call op 0`, if the call is one of the comparing functions of
    /// `string.h`.
    pub(super) fn sign_test(&mut self, call: &Expr, op: BinOp) -> Option<R<syn::Expr>> {
        let ExprKind::Call(callee, args) = &call.kind else {
            return None;
        };
        let name = callee.function()?;
        if self.program.function(name).is_some() {
            return None;
        }
        Some(match (name, args.as_slice()) {
            ("strcmp", [left, right]) => self.compare_texts(left, right, None, op),
            ("strncmp", [left, right, count]) => self.compare_texts(left, right, Some(count), op),
            ("memcmp", [left, right, size]) => self.compare_memory(left, right, size, op, call.at),
            _ => return None,
        })
    }

    /// R30: two texts compared as `strcmp` does, as unsigned bytes up to
    /// the NUL, which is how Rust orders `&str` and `&[u8]` too. With a
    /// count, as `strncmp` does within that many bytes.
    fn compare_texts(
        &mut self,
        left: &Expr,
        right: &Expr,
        count: Option<&Expr>,
        op: BinOp,
    ) -> R<syn::Expr> {
        let ordering = !matches!(op, BinOp::Eq | BinOp::Ne);
        // Two literals compare here, where Clippy refuses `"a" == "a"`.
        if let (Some(mut first), Some(mut second)) = (literal_bytes(left), literal_bytes(right)) {
            let count = match count {
                None => Some(usize::MAX),
                Some(count) => count
                    .constant(self.program)
                    .filter(|count| *count >= 0)
                    .map(|count| usize::try_from(count).unwrap_or(usize::MAX)),
            };
            if let Some(count) = count {
                first = &first[..first.len().min(count)];
                second = &second[..second.len().min(count)];
                return Ok(truth(op, first.cmp(second)));
            }
        }
        // `text == ""`: the text is empty.
        if !ordering && count.is_none() {
            for (empty, text) in [(left, right), (right, left)] {
                if literal_bytes(empty) == Some(&[]) {
                    let text = match self.reads_bytes(text) {
                        true => self.bytes_of(text, false)?,
                        false => self.str_of(text, false)?,
                    };
                    let empty = rust::method(text, "is_empty", []);
                    return Ok(match op {
                        BinOp::Eq => empty,
                        _ => rust::unary("!", empty),
                    });
                }
            }
        }
        let pure = left.is_pure() && right.is_pure();
        // No bytes are equal to no bytes.
        if pure && count.is_some_and(|count| count.constant(self.program) == Some(0)) {
            return Ok(truth(op, std::cmp::Ordering::Equal));
        }
        let helpers = self.helpers.clone();
        // A literal is an array, which orders only against an array of its
        // own length.
        let sliced = ordering;
        let (left, right) = match count {
            None if !self.reads_bytes(left) && !self.reads_bytes(right) => {
                (self.str_of(left, ordering)?, self.str_of(right, ordering)?)
            }
            None => (self.bytes_of(left, sliced)?, self.bytes_of(right, sliced)?),
            Some(count) => {
                // The count is read once for each side.
                if !self.repeatable(count) {
                    return unsupported(
                        "a count of `strncmp` that calls or assigns: no rule yet",
                        count.at,
                    );
                }
                (
                    self.bytes_within(left, count, sliced)?,
                    self.bytes_within(right, count, sliced)?,
                )
            }
        };
        // A text is equal to itself, and Clippy refuses `x == x`.
        let tokens = |side: &syn::Expr| side.to_token_stream().to_string();
        if pure && tokens(&left) == tokens(&right) {
            self.helpers = helpers;
            return Ok(truth(op, std::cmp::Ordering::Equal));
        }
        Ok(rust::binary(left, symbol(op), right))
    }

    /// R31: `memcmp` of two arrays of one element type, as a comparison of
    /// their first elements: in their order for bytes, and only for
    /// equality for wider numbers, whose bytes order differently from their
    /// values.
    fn compare_memory(
        &mut self,
        left: &Expr,
        right: &Expr,
        size: &Expr,
        op: BinOp,
        at: u32,
    ) -> R<syn::Expr> {
        let program = self.program;
        let (left, right) = (left.uncast(), right.uncast());
        let byte = |ty: &Type| {
            matches!(
                ty,
                Type::Char
                    | Type::Int {
                        bits: 8,
                        signed: false
                    }
            )
        };
        let (Some(element), Some(other)) = (left.ty.element(), right.ty.element()) else {
            return unsupported("`memcmp` of something that is no pointer", at);
        };
        // `char` and `unsigned char` are both `u8`.
        let bytes = byte(element) && byte(other);
        if element != other && !bytes {
            return unsupported("`memcmp` of two different types: no rule yet", at);
        }
        // Padding, `-0.0` and NaN make the bytes of other types differ from
        // their values.
        if !element.is_integer() {
            return unsupported("`memcmp` of other than integers: no rule yet", at);
        }
        if !bytes && !matches!(op, BinOp::Eq | BinOp::Ne) {
            return unsupported(
                "an ordering by `memcmp` of numbers wider than a byte: no rule yet",
                at,
            );
        }
        let Some((each, _)) = element.layout(program) else {
            return unsupported("`memcmp` of a type without a size", at);
        };
        let each = i128::from(each);
        let is_each = |bytes: &Expr| bytes.constant(program) == Some(each);
        let count = match (&size.uncast().kind, size.constant(program)) {
            (_, Some(size)) if size >= 0 && size % each == 0 => Ok(size / each),
            _ if bytes => Err(size),
            (ExprKind::Binary(BinOp::Mul, count, factor), _) if is_each(factor) => Err(&**count),
            (ExprKind::Binary(BinOp::Mul, factor, count), _) if is_each(factor) => Err(&**count),
            _ => return unsupported("a size of `memcmp` that is no whole number of elements", at),
        };
        // The size is written once for each side.
        if count.is_err() && !self.repeatable(size) {
            return unsupported("a size of `memcmp` that calls or assigns: no rule yet", at);
        }
        let pure = left.is_pure() && right.is_pure() && size.is_pure();
        if pure && matches!(count, Ok(0)) {
            return Ok(truth(op, std::cmp::Ordering::Equal));
        }
        let is_literal = |side: &Expr| match &side.kind {
            ExprKind::Decay(array) => matches!(array.kind, ExprKind::Str(_)),
            _ => false,
        };
        let whole = |side: &Expr, count: i128| match &side.kind {
            ExprKind::Decay(array) => {
                !is_literal(side)
                    && matches!(array.ty, Type::Array(_, Some(length)) if i128::from(length) == count)
            }
            _ => false,
        };
        let helpers = self.helpers.clone();
        let (left, right) = match count {
            // One element each, which may be a variable of its own.
            Ok(1) if !is_literal(left) && !is_literal(right) => (
                self.pointee(left, false, false)?,
                self.pointee(right, false, false)?,
            ),
            Ok(count) if whole(left, count) && whole(right, count) => {
                (self.slice(left, false)?, self.slice(right, false)?)
            }
            count => {
                let count = match count {
                    Ok(count) => rust::int(count, ""),
                    Err(count) => self.index_value(count)?,
                };
                (
                    self.memory(left, count.clone())?,
                    self.memory(right, count)?,
                )
            }
        };
        let tokens = |side: &syn::Expr| side.to_token_stream().to_string();
        if pure && tokens(&left) == tokens(&right) {
            self.helpers = helpers;
            return Ok(truth(op, std::cmp::Ordering::Equal));
        }
        Ok(rust::binary(left, symbol(op), right))
    }

    /// R31: the first `count` elements that a pointer points to, to read:
    /// `a[..count]`. A string literal is its bytes with the NUL.
    fn memory(&mut self, pointer: &Expr, count: syn::Expr) -> R<syn::Expr> {
        let literal = match &pointer.kind {
            ExprKind::Decay(literal) => match &literal.kind {
                ExprKind::Str(bytes) => Some(bytes),
                _ => None,
            },
            _ => None,
        };
        let slice = match literal {
            Some(bytes) => {
                let mut bytes = bytes.clone();
                bytes.push(0);
                byte_string(&bytes, false)
            }
            None => self.slice(pointer, false)?,
        };
        Ok(rust::index(slice, rust::until(count)))
    }

    /// R30: text that is a `&str` in Rust, to compare with another. A
    /// `String` orders only against a `String`.
    fn str_of(&mut self, text: &Expr, ordering: bool) -> R<syn::Expr> {
        if !self.is_argument(text) {
            return self.pointer(text, Repr::TEXT);
        }
        let argument = self.argument(text)?;
        Ok(match ordering {
            true => rust::method(argument, "as_str", []),
            false => argument,
        })
    }

    /// R30: the text that a pointer points to as bytes, without its NUL:
    /// a byte-string literal, the bytes of a `&str` or of an argument, or
    /// the bytes up to the NUL. `sliced` asks for a slice where a literal
    /// would be an array.
    pub(super) fn bytes_of(&mut self, text: &Expr, sliced: bool) -> R<syn::Expr> {
        if let Some(bytes) = literal_bytes(text) {
            return Ok(byte_string(bytes, sliced));
        }
        if self.is_argument(text) {
            return Ok(rust::method(self.argument(text)?, "as_bytes", []));
        }
        if self.is_text(text) {
            let text = self.pointer(text, Repr::TEXT)?;
            return Ok(rust::method(text, "as_bytes", []));
        }
        self.until_nul(text)
    }

    /// R30: the text that `strncmp` reads within `count` bytes. Bytes from
    /// memory need no NUL among the first `count`, and a `&str` or a
    /// literal ends where it ends.
    fn bytes_within(&mut self, text: &Expr, count: &Expr, sliced: bool) -> R<syn::Expr> {
        let literal = literal_bytes(text);
        if let (Some(bytes), Some(count @ 0..)) = (literal, count.constant(self.program)) {
            let end = bytes
                .len()
                .min(usize::try_from(count).unwrap_or(usize::MAX));
            return Ok(byte_string(&bytes[..end], sliced));
        }
        let ended = match literal {
            Some(bytes) => Some(byte_string(bytes, false)),
            None if self.is_argument(text) => {
                Some(rust::method(self.argument(text)?, "as_bytes", []))
            }
            None if self.is_text(text) => {
                let text = self.pointer(text, Repr::TEXT)?;
                Some(rust::method(text, "as_bytes", []))
            }
            None => None,
        };
        let count = self.index_value(count)?;
        Ok(match ended {
            Some(bytes) => {
                self.helper(Helper::FirstBytes);
                rust::call("first_bytes", [bytes, count])
            }
            None => {
                let bytes = self.pointer(text, Repr::BYTES)?;
                self.helper(Helper::TextWithin);
                rust::call("text_within", [bytes, count])
            }
        })
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
        let target = self.pointer(buffer.uncast(), Repr::BYTES_WRITTEN)?;
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
        self.first_of(pointer, count, true)
    }

    /// `first`, as a place to read when `write` is false.
    fn first_of(
        &mut self,
        pointer: &Expr,
        count: Result<i128, &Expr>,
        write: bool,
    ) -> R<syn::Expr> {
        let count = count.or_else(|count| count.constant(self.program).ok_or(count));
        let whole = match &pointer.uncast().kind {
            ExprKind::Decay(array) => match &array.ty {
                Type::Array(_, Some(length)) => count.ok() == Some(i128::from(*length)),
                _ => false,
            },
            _ => false,
        };
        let slice = self.slice(pointer.uncast(), write)?;
        Ok(match (whole, count) {
            (true, _) => slice,
            (false, Ok(count @ 0..)) => rust::index(slice, rust::until(rust::int(count, ""))),
            (false, Ok(_)) => return unsupported("a negative size", pointer.at),
            (false, Err(count)) => rust::index(slice, rust::until(self.index_value(count)?)),
        })
    }

    /// The first `count` elements that a pointer points to, as a place to
    /// read or to store to; `first` for a pointer that may also be a
    /// string literal.
    fn prefix(&mut self, pointer: &Expr, count: Result<i128, &Expr>, write: bool) -> R<syn::Expr> {
        if write {
            return self.first(pointer, count);
        }
        let ExprKind::Decay(literal) = &pointer.uncast().kind else {
            return self.first_of(pointer, count, false);
        };
        let ExprKind::Str(_) = literal.kind else {
            return self.first_of(pointer, count, false);
        };
        let bytes = self.pointer(pointer, Repr::BYTES)?;
        let count = match count.or_else(|count| count.constant(self.program).ok_or(count)) {
            Ok(count @ 0..) => rust::int(count, ""),
            Ok(_) => return unsupported("a negative size", pointer.at),
            Err(count) => self.index_value(count)?,
        };
        Ok(rust::index(bytes, rust::until(count)))
    }

    /// R29: `memcpy` and `memmove` of elements that assignment copies, as
    /// an assignment of one element, `copy_within` in one array, or
    /// `copy_from_slice` between two.
    fn memcpy(&mut self, target: &Expr, source: &Expr, size: &Expr, at: u32) -> R<syn::Expr> {
        let program = self.program;
        let (target, source) = (target.uncast(), source.uncast());
        let (Some(element), Some(other)) = (target.ty.element(), source.ty.element()) else {
            return unsupported("a copy of something that is no pointer", at);
        };
        // Copying between two types reinterprets memory, which safe Rust
        // does not do.
        if element != other || !self.is_copy(element) {
            return unsupported(
                "a copy between two types, or of a type that is not copied",
                at,
            );
        }
        let each = element.layout(program).map(|(each, _)| i128::from(each));
        let is_each = |bytes: &Expr| each.is_some() && bytes.constant(program) == each;
        let count = match (size.constant(program), &size.uncast().kind, each) {
            // The size of the element's own type is one element, also
            // where the translation does not know its size in C, as for
            // a record with bit-fields.
            (_, ExprKind::SizeOf(ty), _) if ty == element => Ok(1),
            (_, _, None) => return unsupported("a copy of a type without a size", at),
            (Some(bytes), _, Some(each)) if bytes % each == 0 => Ok(bytes / each),
            (Some(_), ..) => return unsupported("a copy of part of an element", at),
            (None, _, Some(1)) => Err(size),
            (None, ExprKind::Binary(BinOp::Mul, count, bytes), _) if is_each(bytes) => {
                Err(&**count)
            }
            (None, ExprKind::Binary(BinOp::Mul, bytes, count), _) if is_each(bytes) => {
                Err(&**count)
            }
            _ => return unsupported("a copy whose size is no count of elements", at),
        };
        // The count is written once for each side.
        if let Err(count) = count
            && !self.repeatable(count)
        {
            return unsupported("a copy whose size calls or assigns: no rule yet", at);
        }
        if count.ok() == Some(1) && (self.points_to_one(target) || self.points_to_one(source)) {
            let value = self.pointee(source, false, false)?;
            return Ok(rust::assign(self.pointee(target, true, false)?, value));
        }
        // Rust borrows one array once, so a copy within it is one call,
        // which is right where the two parts overlap.
        if let (Some((array, to)), Some((other, from))) = (split(target), split(source))
            && same_place(array, other)
        {
            let start = match from {
                Some(from) if !from.is_pure() => {
                    return unsupported("a copy from an offset with effects", at);
                }
                Some(from) => Some(from.constant(program).ok_or(from)),
                None => None,
            };
            let length = match count {
                Ok(count @ 0..) => Ok(count),
                Ok(_) => return unsupported("a negative size", at),
                Err(count) => Err(self.index_value(count)?),
            };
            let range = match (start, length) {
                (None | Some(Ok(0)), Ok(length)) => rust::until(rust::int(length, "")),
                (None | Some(Ok(0)), Err(length)) => rust::until(length),
                (Some(Ok(start)), Ok(length)) => {
                    rust::range(rust::int(start, ""), rust::int(start + length, ""))
                }
                (Some(start), length) => {
                    let start = match start {
                        Ok(start) => rust::int(start, ""),
                        Err(start) => self.index_value(start)?,
                    };
                    let length = match length {
                        Ok(length) => rust::int(length, ""),
                        Err(length) => length,
                    };
                    rust::range(start.clone(), rust::binary(start, "+", length))
                }
            };
            let to = match to {
                Some(to) => self.index_value(to)?,
                None => rust::int(0, ""),
            };
            let array = self.slice(array, true)?;
            return Ok(rust::method(array, "copy_within", [range, to]));
        }
        let value = rust::borrow(false, self.prefix(source, count, false)?);
        Ok(rust::method(
            self.prefix(target, count, true)?,
            "copy_from_slice",
            [value],
        ))
    }

    /// Whether an expression gives the same value and changes nothing
    /// when it is evaluated twice: it assigns nothing and calls only the
    /// library's `strlen`.
    pub(super) fn repeatable(&self, expr: &Expr) -> bool {
        let strlen = self.program.function("strlen").is_none();
        !expr.any(&mut |part| match &part.kind {
            ExprKind::Assign { .. } | ExprKind::IncDec { .. } => true,
            ExprKind::Call(callee, _) => !strlen || callee.function() != Some("strlen"),
            _ => false,
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
        let (text, arguments) = self.format(format, values, false, at)?;
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
    /// prints. `exact` asks for text whose bytes are those that C writes,
    /// to be stored rather than printed.
    fn format(
        &mut self,
        format: &Expr,
        values: &[Expr],
        exact: bool,
        at: u32,
    ) -> R<(String, Vec<syn::Expr>)> {
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
            let (spec, argument) = self.converted(&conversion, value, exact, at)?;
            text.push_str(&spec);
            arguments.push(argument);
        }
        text.push_str(&rest.replace('{', "{{").replace('}', "}}"));
        if values.next().is_some() {
            return unsupported("a `printf` value without a conversion", at);
        }
        Ok((text, arguments))
    }

    /// R27: `sprintf` and `snprintf`, with the size of `snprintf`, as the
    /// text of `format!` stored by a helper that returns what C does. The
    /// text comes first, so that its values may read the target.
    fn sprintf(
        &mut self,
        target: &Expr,
        size: Option<&Expr>,
        format: &Expr,
        values: &[Expr],
        at: u32,
    ) -> R<syn::Expr> {
        let (text, arguments) = self.format(format, values, true, at)?;
        let text: syn::Expr = match arguments.is_empty() {
            true => rust::string(&text.replace("{{", "{").replace("}}", "}")),
            false => {
                let text = rust::string(&text);
                parse_quote!(&format!(#text #(, #arguments)*))
            }
        };
        let bytes = Repr {
            mutable: true,
            ..Repr::BYTES
        };
        let Some(size) = size else {
            let target = self.pointer(target.uncast(), bytes)?;
            self.helper(Helper::StoreText);
            return Ok(rust::call("store_text", [text, target]));
        };
        if let (Some(count), Some(room)) = (size.constant(self.program), self.room(target)) {
            if count > room {
                return unsupported("an `snprintf` size beyond its buffer", at);
            }
            let target = rust::borrow(true, self.first(target, Ok(count))?);
            self.helper(Helper::StoreTextCut);
            return Ok(rust::call("store_text_cut", [text, target]));
        }
        // As for `fgets`, a size beyond the buffer is no error in C until
        // the text needs the room.
        let target = self.pointer(target.uncast(), bytes)?;
        let size = self.expr(size)?;
        self.helper(Helper::StoreTextWithin);
        Ok(rust::call("store_text_within", [text, target, size]))
    }

    /// Rust's format for one conversion, and the value it prints.
    fn converted(
        &mut self,
        conversion: &Conversion,
        value: &Expr,
        exact: bool,
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
                match (value.ty == read, round_trip(value, &read)) {
                    (true, _) => self.expr(value)?,
                    (false, Some(original)) => self.expr(original)?,
                    (false, None) => rust::cast(self.expr(value)?, self.ty(&read, at)?),
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
            // `as char` of a byte above 127 is two bytes of UTF-8.
            b'c' if exact && !matches!(value.constant(self.program), Some(0..=127)) => {
                return unsupported(
                    "`%c` of a value that may be above 127 into bytes: no rule yet",
                    at,
                );
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
                    // `text_of` writes bytes that are not UTF-8 as U+FFFD.
                    None if exact && self.reads_bytes(value) => {
                        return unsupported("`%s` of bytes into bytes: no rule yet", at);
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

/// The pointer at the root of a pointer into an array, and the offset
/// from it: `a` and `i` for `a + i` and `&a[i]`, `p` alone for `p`.
fn split(pointer: &Expr) -> Option<(&Expr, Option<&Expr>)> {
    let pointer = pointer.uncast();
    let (base, offset) = match &pointer.kind {
        ExprKind::Binary(BinOp::Add, left, right) => match left.ty.is_pointer() {
            true => (left.uncast(), Some(&**right)),
            false => (right.uncast(), Some(&**left)),
        },
        ExprKind::AddrOf(place) => match &place.kind {
            ExprKind::Index(base, index) => (base.uncast(), Some(&**index)),
            _ => return None,
        },
        _ => (pointer, None),
    };
    matches!(
        base.kind,
        ExprKind::Decay(_) | ExprKind::Var(_) | ExprKind::Member { .. }
    )
    .then_some((base, offset))
}

/// Whether two expressions name one place, wherever they are evaluated in
/// one statement: the same variable, field or element by a constant or
/// by the same variable.
fn same_place(one: &Expr, other: &Expr) -> bool {
    match (&one.uncast().kind, &other.uncast().kind) {
        (ExprKind::Var(one), ExprKind::Var(other)) => one == other,
        (ExprKind::Decay(one), ExprKind::Decay(other))
        | (ExprKind::Deref(one), ExprKind::Deref(other)) => same_place(one, other),
        (
            ExprKind::Member {
                base: one,
                field,
                arrow,
                ..
            },
            ExprKind::Member {
                base: other,
                field: other_field,
                arrow: other_arrow,
                ..
            },
        ) => field == other_field && arrow == other_arrow && same_place(one, other),
        (ExprKind::Index(one, index), ExprKind::Index(other, other_index)) => {
            let same_index = match (&index.uncast().kind, &other_index.uncast().kind) {
                (ExprKind::Int(index), ExprKind::Int(other)) => index == other,
                (ExprKind::Var(index), ExprKind::Var(other)) => index == other,
                _ => false,
            };
            same_index && same_place(one, other)
        }
        _ => false,
    }
}

/// Whether `op` holds of two values in the order given, as `true` or
/// `false`.
fn truth(op: BinOp, order: std::cmp::Ordering) -> syn::Expr {
    let holds = match op {
        BinOp::Eq => order.is_eq(),
        BinOp::Ne => order.is_ne(),
        BinOp::Lt => order.is_lt(),
        BinOp::Gt => order.is_gt(),
        BinOp::Le => order.is_le(),
        _ => order.is_ge(),
    };
    match holds {
        true => parse_quote!(true),
        false => parse_quote!(false),
    }
}

/// The bytes of a string literal up to its first NUL, where C's readers of
/// text stop.
fn literal_bytes(text: &Expr) -> Option<&[u8]> {
    let ExprKind::Decay(literal) = &text.uncast().kind else {
        return None;
    };
    let ExprKind::Str(bytes) = &literal.kind else {
        return None;
    };
    bytes.split(|&byte| byte == 0).next()
}

/// A byte-string literal, `b"fast"`, or a slice of it, `b"fast".as_slice()`.
fn byte_string(bytes: &[u8], sliced: bool) -> syn::Expr {
    let literal = syn::LitByteStr::new(bytes, proc_macro2::Span::call_site());
    match sliced {
        true => parse_quote!(#literal.as_slice()),
        false => parse_quote!(#literal),
    }
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
