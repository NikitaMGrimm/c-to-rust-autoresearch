//! R40: the searches of the library, `strchr`, `strrchr`, `memchr`,
//! `strstr` and `strpbrk`, as the `Option<usize>` of the place they found
//! in their first argument, an index into its bytes.

use super::{Helper, Lower, R, unsupported};
use crate::c::{Expr, ExprKind, Type};
use crate::rust;
use syn::parse_quote;

impl Lower<'_> {
    /// The search as an `Option<usize>`.
    pub(super) fn search(&mut self, call: &Expr) -> R<syn::Expr> {
        let at = call.at;
        let ExprKind::Call(callee, args) = &call.kind else {
            return unsupported("a search with no rule", at);
        };
        let name = callee.function().unwrap_or_default();
        // R51: a search of the program's own gives the place itself.
        if self.program.function(name).is_some() {
            return self.call(call, callee, args);
        }
        if let Some((text, letter)) = self.text_and_char(name, args)? {
            let method = if name == "strrchr" { "rfind" } else { "find" };
            return Ok(rust::method(text, method, [letter]));
        }
        match (name, args.as_slice()) {
            ("strchr" | "strrchr", [text, byte]) => {
                let text = self.bytes_of(text, false)?;
                let byte = self.byte(byte)?;
                let (helper, function) = match name {
                    "strchr" => (Helper::FindByte, "find_byte"),
                    _ => (Helper::RfindByte, "rfind_byte"),
                };
                self.helper(helper);
                Ok(rust::call(function, [text, byte]))
            }
            ("memchr", [bytes, byte, count]) => {
                if !self.reads_bytes(bytes.uncast()) {
                    return unsupported("`memchr` of text: no rule yet", at);
                }
                // C compares bytes, and Rust would compare elements.
                if !matches!(
                    bytes.uncast().ty.element(),
                    Some(
                        Type::Char
                            | Type::Int {
                                bits: 8,
                                signed: false
                            }
                    )
                ) {
                    return unsupported("`memchr` of other than bytes: no rule yet", at);
                }
                // The byte is compared once for each byte that is read.
                if !byte.is_pure() {
                    return unsupported("`memchr` of a byte with a side effect: no rule yet", at);
                }
                let bytes = rust::method(self.slice(bytes, false)?, "iter", []);
                let taken = rust::method(bytes, "take", [self.index_value(count)?]);
                let byte = self.byte(byte)?;
                Ok(rust::method(
                    taken,
                    "position",
                    [parse_quote!(|&found| found == #byte)],
                ))
            }
            ("strstr", [text, part]) if !self.reads_bytes(text) && !self.reads_bytes(part) => {
                let (text, part) = (self.text(text, false)?, self.text(part, true)?);
                Ok(rust::method(text, "find", [part]))
            }
            ("strstr" | "strpbrk", [text, part]) => {
                let (text, part) = (self.bytes_of(text, false)?, self.bytes_of(part, false)?);
                let (helper, function) = match name {
                    "strstr" => (Helper::FindBytes, "find_bytes"),
                    _ => (Helper::FindAny, "find_any"),
                };
                self.helper(helper);
                Ok(rust::call(function, [text, part]))
            }
            _ => unsupported(
                format!("this call of the library's `{name}`: no rule yet"),
                at,
            ),
        }
    }

    /// Whether the search finds something, for a test: `contains` where
    /// text is searched for a character or for other text.
    pub(super) fn search_found(&mut self, call: &Expr) -> R<syn::Expr> {
        if let ExprKind::Call(callee, args) = &call.kind {
            let name = callee.function().unwrap_or_default();
            if let Some((text, letter)) = self.text_and_char(name, args)? {
                return Ok(rust::method(text, "contains", [letter]));
            }
            if let ("strstr", [text, part]) = (name, args.as_slice())
                && !self.reads_bytes(text)
                && !self.reads_bytes(part)
            {
                let (text, part) = (self.text(text, false)?, self.text(part, true)?);
                return Ok(rust::method(text, "contains", [part]));
            }
        }
        Ok(rust::method(self.search(call)?, "is_some", []))
    }

    /// The text and the character of `strchr` or `strrchr` that searches
    /// text for a constant ASCII character other than NUL, which a `&str`
    /// finds by itself.
    fn text_and_char(&mut self, name: &str, args: &[Expr]) -> R<Option<(syn::Expr, syn::Expr)>> {
        let ("strchr" | "strrchr", [text, byte]) = (name, args) else {
            return Ok(None);
        };
        let Some(value) = byte.constant(self.program) else {
            return Ok(None);
        };
        // C converts the character to a `char`.
        let letter = value as u8;
        if self.reads_bytes(text) || !(1..0x80).contains(&letter) {
            return Ok(None);
        }
        let text = self.text(text, false)?;
        Ok(Some((text, rust::char(char::from(letter)))))
    }

    /// The character that a search looks for, as the byte that C converts
    /// it to.
    fn byte(&mut self, value: &Expr) -> R<syn::Expr> {
        if let Some(value) = value.constant(self.program) {
            return Ok(rust::byte(value as u8));
        }
        Ok(match &value.kind {
            // A `char` that C widened to `int` is the byte itself.
            ExprKind::Cast(inner) if inner.ty == Type::Char => self.expr(inner)?,
            _ => rust::cast(self.expr(value)?, parse_quote!(u8)),
        })
    }
}
