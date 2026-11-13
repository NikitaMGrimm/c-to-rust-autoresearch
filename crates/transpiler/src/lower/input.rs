//! Rules for reading standard input. C reads it a byte at a time and leaves
//! what a conversion does not take, so the translation does too, through
//! helpers over `std::io::stdin().lock()`.

use super::{Helper, Lower, R, unsupported};
use crate::c::{BinOp, Expr, ExprKind, Type};
use crate::rust;
use syn::parse_quote;

/// One directive of a `scanf` format.
pub(super) enum Read<'e> {
    /// White space in the format: skips the white space of the input.
    Space,
    /// A conversion: the helper that reads it and the pointer it stores
    /// through.
    Value { helper: Helper, target: &'e Expr },
}

/// Whether the pointer is to the same place whatever the program stores:
/// a variable or one of its parts, with no index that is computed.
fn is_fixed(pointer: &Expr) -> bool {
    match &pointer.kind {
        ExprKind::Var(_) => true,
        ExprKind::Cast(inner) | ExprKind::AddrOf(inner) | ExprKind::Decay(inner) => is_fixed(inner),
        ExprKind::Member { base, .. } => is_fixed(base),
        ExprKind::Index(base, index) => is_fixed(base) && matches!(index.kind, ExprKind::Int(_)),
        _ => false,
    }
}

/// The name of what a conversion read, in the translation.
const SCANNED: &str = "scanned";

impl<'e> Lower<'_> {
    /// R11: the directives of a call of `scanf`, or of `fscanf` on `stdin`,
    /// if the expression is one.
    pub(super) fn scan(&self, call: &'e Expr) -> R<Option<Vec<Read<'e>>>> {
        let ExprKind::Call(callee, args) = &call.kind else {
            return Ok(None);
        };
        let (format, targets) = match (callee.function(), args.as_slice()) {
            (Some("scanf"), [format, targets @ ..]) => (format, targets),
            (Some("fscanf"), [stream, format, targets @ ..]) if matches!(&stream.uncast().kind, ExprKind::Extern(name) if name == "stdin") => {
                (format, targets)
            }
            _ => return Ok(None),
        };
        let at = call.at;
        let format = match &format.uncast().kind {
            ExprKind::Decay(literal) => match &literal.kind {
                ExprKind::Str(format) => format.as_slice(),
                _ => return unsupported("`scanf` with a format that is not a literal", at),
            },
            _ => return unsupported("`scanf` with a format that is not a literal", at),
        };
        let mut reads = Vec::new();
        let mut targets = targets.iter();
        let mut rest = format;
        while let Some((&first, after)) = rest.split_first() {
            rest = after;
            if first == b' ' || (b'\t'..=b'\r').contains(&first) {
                if !matches!(reads.last(), Some(Read::Space)) {
                    reads.push(Read::Space);
                }
                continue;
            }
            if first != b'%' {
                return unsupported("text in a `scanf` format: no rule yet", at);
            }
            let length = rest
                .iter()
                .position(|byte| !matches!(byte, b'h' | b'l'))
                .unwrap_or(rest.len());
            let (length, after) = rest.split_at(length);
            let Some((&letter, after)) = after.split_first() else {
                return unsupported("a `scanf` conversion with no rule", at);
            };
            rest = after;
            let (helper, bits) = match (letter, length) {
                (b'd' | b'u', b"hh") => (Helper::ScanInt, 8),
                (b'd' | b'u', b"h") => (Helper::ScanInt, 16),
                (b'd' | b'u', b"") => (Helper::ScanInt, 32),
                // `scan_int` stops at the limits of `long`, as C does for
                // `%ld`; `%lu` reads further.
                (b'd', b"l" | b"ll") => (Helper::ScanInt, 64),
                (b'c', b"") => (Helper::ReadByte, 8),
                _ => {
                    return unsupported(
                        format!(
                            "the `scanf` conversion `%{}{}`: no rule yet",
                            String::from_utf8_lossy(length),
                            char::from(letter)
                        ),
                        at,
                    );
                }
            };
            let Some(target) = targets.next() else {
                return unsupported("a `scanf` conversion without a pointer", at);
            };
            // The conversion stores a value of its own size, whatever the
            // pointer's type is.
            if target
                .ty
                .element()
                .and_then(Type::int)
                .map(|(bits, _)| bits)
                != Some(bits)
            {
                return unsupported("a `scanf` conversion for a variable of another size", at);
            }
            // C has every pointer before it reads, and the translation
            // only when it stores: the same where finding the pointer
            // changes nothing and no earlier store moves it.
            let later = reads.iter().any(|read| matches!(read, Read::Value { .. }));
            if !target.is_pure() || later && !is_fixed(target) {
                return unsupported(
                    "a `scanf` pointer that the call itself may change: no rule yet",
                    at,
                );
            }
            // Numbers skip white space themselves.
            if helper == Helper::ScanInt && matches!(reads.last(), Some(Read::Space)) {
                reads.pop();
            }
            reads.push(Read::Value { helper, target });
        }
        if targets.next().is_some() {
            return unsupported("a `scanf` pointer without a conversion", at);
        }
        if !reads.iter().any(|read| matches!(read, Read::Value { .. })) {
            return unsupported("`scanf` without a conversion: no rule yet", at);
        }
        Ok(Some(reads))
    }

    /// The call of a conversion's helper, and the statement that stores
    /// what it read.
    fn scanned(&mut self, helper: Helper, target: &Expr) -> R<(syn::Expr, syn::Stmt)> {
        self.helper(helper);
        let call = match helper {
            Helper::ReadByte => rust::call("read_byte", []),
            _ => rust::call("scan_int", []),
        };
        let read = match helper {
            Helper::ReadByte => Type::Char,
            _ => Type::Int {
                bits: 64,
                signed: true,
            },
        };
        let element = target.ty.element().unwrap_or(&Type::Void);
        let unsigned_byte = Type::Int {
            bits: 8,
            signed: false,
        };
        let value = match *element == read || (read == Type::Char && *element == unsigned_byte) {
            true => rust::name(SCANNED),
            false => rust::cast(rust::name(SCANNED), self.ty(element, target.at)?),
        };
        let place = self.pointee(target, true, false)?;
        let store = rust::assign(place, value);
        Ok((call, parse_quote!(#store;)))
    }

    /// The reads in order, each inside the one before it, since `scanf`
    /// stops at the first conversion that finds nothing. `inner` runs after
    /// the last, and `other` where a conversion finds nothing.
    pub(super) fn scan_chain(
        &mut self,
        reads: &[Read],
        mut inner: Vec<syn::Stmt>,
        other: Option<&syn::Block>,
    ) -> R<Vec<syn::Stmt>> {
        let name = rust::ident(SCANNED);
        for read in reads.iter().rev() {
            match read {
                Read::Space => {
                    self.helper(Helper::SkipSpace);
                    inner.insert(0, parse_quote!(skip_space();));
                }
                Read::Value { helper, target } => {
                    let (call, store) = self.scanned(*helper, target)?;
                    let other = other.iter();
                    inner = vec![syn::Stmt::Expr(
                        parse_quote! {
                            if let Some(#name) = #call {
                                #store
                                #(#inner)*
                            } #(else #other)*
                        },
                        None,
                    )];
                }
            }
        }
        Ok(inner)
    }

    /// `let Some(scanned) = … else { otherwise };` and the store, for a
    /// call with one conversion whose failure leaves the block.
    pub(super) fn scan_or_leave(
        &mut self,
        reads: &[Read],
        otherwise: syn::Block,
    ) -> R<Option<Vec<syn::Stmt>>> {
        let [Read::Value { helper, target }, after @ ..] = reads else {
            return Ok(None);
        };
        if !after.iter().all(|read| matches!(read, Read::Space)) {
            return Ok(None);
        }
        let name = rust::ident(SCANNED);
        let (call, store) = self.scanned(*helper, target)?;
        let mut stmts = vec![
            parse_quote!(let Some(#name) = #call else #otherwise;),
            store,
        ];
        stmts.extend(self.scan_chain(after, Vec::new(), None)?);
        Ok(Some(stmts))
    }

    /// The call of `scanf` whose result the condition compares with its
    /// number of conversions, and whether the condition holds when all of
    /// them found a value. No other use of the result has a rule: it is
    /// also -1 at the end of the input, which the helpers do not tell from
    /// a conversion that found nothing.
    pub(super) fn scan_test(&self, cond: &'e Expr) -> R<Option<(Vec<Read<'e>>, bool)>> {
        let ExprKind::Binary(op, left, right) = &cond.kind else {
            return Ok(None);
        };
        let (reads, op, count) = match (self.scan(left)?, self.scan(right)?) {
            (Some(reads), None) => (reads, *op, right),
            (None, Some(reads)) => {
                let flipped = match op {
                    BinOp::Lt => BinOp::Gt,
                    BinOp::Gt => BinOp::Lt,
                    BinOp::Le => BinOp::Ge,
                    BinOp::Ge => BinOp::Le,
                    op => *op,
                };
                (reads, flipped, left)
            }
            _ => return Ok(None),
        };
        let all = reads
            .iter()
            .filter(|read| matches!(read, Read::Value { .. }))
            .count() as i128;
        let found = match (op, count.constant(self.program)) {
            (BinOp::Eq | BinOp::Ge, Some(count)) if count == all => true,
            (BinOp::Ne | BinOp::Lt, Some(count)) if count == all => false,
            (BinOp::Gt, Some(count)) if count == all - 1 => true,
            (BinOp::Le, Some(count)) if count == all - 1 => false,
            _ => {
                return unsupported(
                    "the result of `scanf` other than compared with its number of conversions",
                    cond.at,
                );
            }
        };
        Ok(Some((reads, found)))
    }

    /// Whether every conversion found a value, or whether one did not, as
    /// a `bool`.
    pub(super) fn scan_cond(&mut self, reads: &[Read], found: bool) -> R<syn::Expr> {
        let (then, other): (syn::Expr, syn::Block) = match found {
            true => (parse_quote!(true), parse_quote!({ false })),
            false => (parse_quote!(false), parse_quote!({ true })),
        };
        let mut chain = self.scan_chain(reads, vec![syn::Stmt::Expr(then, None)], Some(&other))?;
        Ok(match (chain.pop(), chain.is_empty()) {
            (Some(syn::Stmt::Expr(test, None)), true) => test,
            // White space is skipped before the first conversion.
            (Some(last), _) => parse_quote!({ #(#chain)* #last }),
            (None, _) => return unsupported("`scanf` without a conversion: no rule yet", 0),
        })
    }
}
