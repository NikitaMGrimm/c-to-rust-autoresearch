//! The translation rules. Each turns one C construct into Rust syntax,
//! reading the facts for whatever depends on the rest of the program.
//! `CATALOGUE.md` lists the rules; what has none is `Unsupported`.

mod expr;
mod function;
mod input;
mod library;
mod pointer;
mod search;
mod stmt;
mod types;

use crate::Unsupported;
use crate::c::{Expr, ExprKind, FuncId, Program, Stmt, Type, VarId, any_expr};
use crate::facts::{Access, Facts, Slot};
use crate::names::Names;
use crate::rust;
use std::collections::BTreeSet;
use syn::parse_quote;

type R<T> = Result<T, Unsupported>;

fn unsupported<T>(what: impl Into<String>, at: u32) -> R<T> {
    Err(Unsupported::new(what, at))
}

/// A function that the translation adds once, when it calls it: one C
/// library function's behaviour on Rust types, where the standard library
/// has no function that behaves the same on every input.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Helper {
    ParseInt,
    ParseIntPrefix,
    ParseUint,
    ParseUintPrefix,
    ParseFloat,
    ParseFloatPrefix,
    ParseIntChecked,
    ParseUintChecked,
    ParseFloatChecked,
    FormatE,
    FormatG,
    PeekByte,
    ReadByte,
    SkipSpace,
    ScanInt,
    ScanError,
    ScanIntOrEnd,
    ReadLine,
    ReadLineWithin,
    UntilNul,
    TextOf,
    CopyText,
    StoreText,
    StoreTextCut,
    StoreTextWithin,
    CopyString,
    AppendString,
    AppendAtMost,
    TextWithin,
    FirstBytes,
    FindByte,
    RfindByte,
    FindBytes,
    FindAny,
}

impl Helper {
    /// The helpers that this one calls.
    fn needs(self) -> &'static [Helper] {
        match self {
            Helper::ParseInt
            | Helper::ParseUint
            | Helper::ParseFloatPrefix
            | Helper::FormatE
            | Helper::CopyText
            | Helper::StoreText
            | Helper::StoreTextCut
            | Helper::TextWithin
            | Helper::FirstBytes
            | Helper::FindByte
            | Helper::RfindByte
            | Helper::FindBytes
            | Helper::FindAny
            | Helper::PeekByte
            | Helper::ScanError
            | Helper::UntilNul => &[],
            Helper::ReadByte | Helper::SkipSpace => &[Helper::PeekByte],
            Helper::ScanInt => &[Helper::SkipSpace, Helper::ReadByte],
            Helper::ScanIntOrEnd => &[Helper::ScanError, Helper::ScanInt],
            Helper::ReadLine => &[Helper::ReadByte],
            Helper::ReadLineWithin => &[Helper::ReadLine, Helper::PeekByte],
            Helper::TextOf | Helper::CopyString | Helper::AppendAtMost => &[Helper::UntilNul],
            Helper::AppendString => &[Helper::UntilNul, Helper::CopyString],
            Helper::ParseIntPrefix => &[Helper::ParseInt],
            Helper::ParseUintPrefix => &[Helper::ParseUint],
            Helper::ParseFloat => &[Helper::ParseFloatPrefix],
            Helper::ParseIntChecked => &[Helper::ParseIntPrefix],
            Helper::ParseUintChecked => &[Helper::ParseUintPrefix],
            Helper::ParseFloatChecked => &[Helper::ParseFloatPrefix],
            Helper::FormatG => &[Helper::FormatE],
            Helper::StoreTextWithin => &[Helper::StoreTextCut],
        }
    }

    /// Whether it calls the methods of `std::io::BufRead`.
    fn reads_input(self) -> bool {
        matches!(self, Helper::PeekByte | Helper::ReadByte)
    }

    fn item(self) -> syn::Item {
        match self {
            Helper::ParseInt => parse_quote! {
                /// The number at the start of `text` as C's `strtoll` reads it in
                /// base 10: white space, a sign, then digits. It is 0 without
                /// digits and the nearest limit when out of range.
                fn parse_int(text: &str) -> i64 {
                    let text = text.trim_start_matches(|c| c == ' ' || ('\t'..='\r').contains(&c));
                    let (negative, digits) = match text.strip_prefix('-') {
                        Some(digits) => (true, digits),
                        None => (false, text.strip_prefix('+').unwrap_or(text)),
                    };
                    let mut value: i64 = 0;
                    for digit in digits.bytes().take_while(u8::is_ascii_digit) {
                        let digit = i64::from(digit - b'0');
                        value = value.saturating_mul(10);
                        value = if negative {
                            value.saturating_sub(digit)
                        } else {
                            value.saturating_add(digit)
                        };
                    }
                    value
                }
            },
            Helper::ParseIntPrefix => parse_quote! {
                /// As `parse_int`, with the index in `text` where the number
                /// ends, as C's `strtoll` sets its end pointer: 0 if the text
                /// starts with no number.
                fn parse_int_prefix(text: &str) -> (i64, usize) {
                    let trimmed = text.trim_start_matches(|c| c == ' ' || ('\t'..='\r').contains(&c));
                    let unsigned = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
                    let digits = unsigned.bytes().take_while(u8::is_ascii_digit).count();
                    match digits {
                        0 => (0, 0),
                        _ => (parse_int(text), text.len() - unsigned.len() + digits),
                    }
                }
            },
            Helper::ParseUint => parse_quote! {
                /// The number at the start of `text` as C's `strtoull` reads it
                /// in base 10: white space, a sign, then digits. It is 0 without
                /// digits, the greatest value when out of range, and counted
                /// down from zero after a `-`.
                fn parse_uint(text: &str) -> u64 {
                    let text = text.trim_start_matches(|c| c == ' ' || ('\t'..='\r').contains(&c));
                    let (negative, digits) = match text.strip_prefix('-') {
                        Some(digits) => (true, digits),
                        None => (false, text.strip_prefix('+').unwrap_or(text)),
                    };
                    let mut value = Some(0u64);
                    for digit in digits.bytes().take_while(u8::is_ascii_digit) {
                        value = value
                            .and_then(|value| value.checked_mul(10))
                            .and_then(|value| value.checked_add(u64::from(digit - b'0')));
                    }
                    match value {
                        Some(value) if negative => value.wrapping_neg(),
                        Some(value) => value,
                        None => u64::MAX,
                    }
                }
            },
            Helper::ParseUintPrefix => parse_quote! {
                /// As `parse_uint`, with the index in `text` where the number
                /// ends, as C's `strtoull` sets its end pointer: 0 if the text
                /// starts with no number.
                fn parse_uint_prefix(text: &str) -> (u64, usize) {
                    let trimmed = text.trim_start_matches(|c| c == ' ' || ('\t'..='\r').contains(&c));
                    let unsigned = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
                    let digits = unsigned.bytes().take_while(u8::is_ascii_digit).count();
                    match digits {
                        0 => (0, 0),
                        _ => (parse_uint(text), text.len() - unsigned.len() + digits),
                    }
                }
            },
            Helper::ParseFloat => parse_quote! {
                /// The number at the start of `text` as C's `strtod` reads a
                /// decimal number. It is 0 without a number.
                fn parse_float<T: std::str::FromStr + Default>(text: &str) -> T {
                    parse_float_prefix(text).0
                }
            },
            Helper::ParseFloatPrefix => parse_quote! {
                /// The number at the start of `text` as C's `strtod` reads a
                /// decimal number: white space, a sign, then digits with a
                /// point and an exponent, or `inf` or `nan`, and the index in
                /// `text` where it ends. It is 0 and 0 without a number.
                fn parse_float_prefix<T: std::str::FromStr + Default>(text: &str) -> (T, usize) {
                    let trimmed = text.trim_start_matches(|c| c == ' ' || ('\t'..='\r').contains(&c));
                    let skipped = text.len() - trimmed.len();
                    let bytes = trimmed.as_bytes();
                    let digits = |from: usize| {
                        bytes[from..]
                            .iter()
                            .take_while(|byte| byte.is_ascii_digit())
                            .count()
                    };
                    let starts = |from: usize, word: &str| {
                        bytes[from..]
                            .get(..word.len())
                            .is_some_and(|start| start.eq_ignore_ascii_case(word.as_bytes()))
                    };
                    let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
                    if let Some(word) = ["infinity", "inf", "nan"]
                        .into_iter()
                        .find(|word| starts(end, word))
                    {
                        let value = trimmed[..end + word.len()].parse().unwrap_or_default();
                        end += word.len();
                        // `nan(chars)` is a NaN too.
                        if word == "nan" && bytes.get(end) == Some(&b'(') {
                            let chars = bytes[end + 1..]
                                .iter()
                                .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
                                .count();
                            if bytes.get(end + 1 + chars) == Some(&b')') {
                                end += chars + 2;
                            }
                        }
                        return (value, skipped + end);
                    }
                    let whole = digits(end);
                    end += whole;
                    let fraction = match bytes.get(end) {
                        Some(b'.') => digits(end + 1),
                        _ => 0,
                    };
                    if whole + fraction == 0 {
                        return (T::default(), 0);
                    }
                    if bytes.get(end) == Some(&b'.') {
                        end += 1 + fraction;
                    }
                    if matches!(bytes.get(end), Some(b'e' | b'E')) {
                        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
                        let exponent = digits(end + 1 + sign);
                        if exponent > 0 {
                            end += 1 + sign + exponent;
                        }
                    }
                    (trimmed[..end].parse().unwrap_or_default(), skipped + end)
                }
            },
            Helper::ParseIntChecked => parse_quote! {
                /// As `parse_int_prefix`, and whether the number is beyond
                /// `i64`, where C's `strtoll` sets `errno` to `ERANGE`.
                fn parse_int_checked(text: &str) -> (i64, usize, bool) {
                    let (value, end) = parse_int_prefix(text);
                    let number = text[..end].trim_start_matches(|c| c == ' ' || ('\t'..='\r').contains(&c));
                    (value, end, end > 0 && number.parse::<i64>().is_err())
                }
            },
            Helper::ParseUintChecked => parse_quote! {
                /// As `parse_uint_prefix`, and whether the digits are beyond
                /// `u64`, where C's `strtoull` sets `errno` to `ERANGE`; a `-`
                /// before them is no error.
                fn parse_uint_checked(text: &str) -> (u64, usize, bool) {
                    let (value, end) = parse_uint_prefix(text);
                    let digits = text[..end].trim_start_matches(|c: char| !c.is_ascii_digit());
                    (value, end, end > 0 && digits.parse::<u64>().is_err())
                }
            },
            Helper::ParseFloatChecked => parse_quote! {
                /// As `parse_float_prefix`, and whether glibc's `strtod` sets
                /// `errno` to `ERANGE`: a number that is infinite or, with a
                /// digit that is not 0, below `smallest`, the least normal
                /// number of `T`. The words `inf` and `nan` are no error.
                fn parse_float_checked<T: std::str::FromStr + Default + Copy + Into<f64>>(
                    text: &str,
                    smallest: f64,
                ) -> (T, usize, bool) {
                    let (value, end) = parse_float_prefix::<T>(text);
                    let number = &text[..end];
                    let word = number.contains(['n', 'N']);
                    let digit = number
                        .bytes()
                        .take_while(|byte| !matches!(byte, b'e' | b'E'))
                        .any(|byte| (b'1'..=b'9').contains(&byte));
                    let wide: f64 = value.into();
                    let range = !word && (wide.is_infinite() || digit && wide.abs() < smallest);
                    (value, end, range)
                }
            },
            Helper::FormatE => parse_quote! {
                /// `value` as C's `%e` prints it with `precision` digits after
                /// the point: an exponent with a sign and at least two digits.
                fn format_e(value: f64, precision: usize) -> String {
                    if !value.is_finite() {
                        let sign = if value.is_sign_negative() { "-" } else { "" };
                        let name = if value.is_nan() { "nan" } else { "inf" };
                        return format!("{sign}{name}");
                    }
                    let text = format!("{value:.precision$e}");
                    let (digits, exponent) = text.split_once('e').unwrap_or((&text, "0"));
                    let exponent: i32 = exponent.parse().unwrap_or(0);
                    let sign = if exponent < 0 { '-' } else { '+' };
                    format!("{digits}e{sign}{:02}", exponent.abs())
                }
            },
            Helper::FormatG => parse_quote! {
                /// `value` as C's `%g` prints it with `precision` significant
                /// digits: as `%e` does when the exponent is below -4 or not
                /// below `precision`, and without an exponent otherwise, in
                /// both cases without the zeros at the end of the fraction.
                fn format_g(value: f64, precision: usize) -> String {
                    let precision = precision.max(1);
                    let scientific = format_e(value, precision - 1);
                    let Some((digits, exponent)) = scientific.split_once('e') else {
                        return scientific;
                    };
                    let trimmed = |digits: &str| match digits.contains('.') {
                        true => digits.trim_end_matches('0').trim_end_matches('.').to_owned(),
                        false => digits.to_owned(),
                    };
                    let power: i32 = exponent.parse().unwrap_or(0);
                    if power < -4 || power >= precision as i32 {
                        return format!("{}e{exponent}", trimmed(digits));
                    }
                    let places = (precision as i32 - 1 - power) as usize;
                    trimmed(&format!("{value:.places$}"))
                }
            },
            Helper::PeekByte => parse_quote! {
                /// The next byte of standard input, which stays there, or `None`
                /// at its end.
                fn peek_byte() -> Option<u8> {
                    std::io::stdin().lock().fill_buf().ok()?.first().copied()
                }
            },
            Helper::ReadByte => parse_quote! {
                /// Takes the next byte of standard input, or returns `None` at
                /// its end.
                fn read_byte() -> Option<u8> {
                    let byte = peek_byte()?;
                    std::io::stdin().lock().consume(1);
                    Some(byte)
                }
            },
            Helper::SkipSpace => parse_quote! {
                /// Takes the white space at the start of standard input, as a
                /// space in a `scanf` format does.
                fn skip_space() {
                    while peek_byte().is_some_and(|byte| byte == b' ' || (b'\t'..=b'\r').contains(&byte)) {
                        std::io::stdin().lock().consume(1);
                    }
                }
            },
            Helper::ScanInt => parse_quote! {
                /// Reads a number from standard input as C's `scanf("%ld")` does:
                /// white space, a sign, then digits, with the nearest limit for
                /// a number out of range. Without a digit it returns `None` and
                /// leaves what it did not read.
                fn scan_int() -> Option<i64> {
                    skip_space();
                    let negative = match peek_byte()? {
                        sign @ (b'-' | b'+') => {
                            read_byte();
                            sign == b'-'
                        }
                        _ => false,
                    };
                    let mut value: Option<i64> = None;
                    while let Some(digit @ b'0'..=b'9') = peek_byte() {
                        read_byte();
                        let digit = i64::from(digit - b'0');
                        let shifted = value.unwrap_or(0).saturating_mul(10);
                        value = Some(if negative {
                            shifted.saturating_sub(digit)
                        } else {
                            shifted.saturating_add(digit)
                        });
                    }
                    value
                }
            },
            Helper::ScanError => parse_quote! {
                /// Why `scanf` stored nothing: the input ended before the
                /// conversion, or what is there is no number.
                enum ScanError {
                    End,
                    NoMatch,
                }
            },
            Helper::ScanIntOrEnd => parse_quote! {
                /// As `scan_int`, and tells the end of standard input after
                /// the white space from text that is no number, as the result
                /// of `scanf` does.
                fn scan_int_or_end() -> Result<i64, ScanError> {
                    skip_space();
                    if peek_byte().is_none() {
                        return Err(ScanError::End);
                    }
                    scan_int().ok_or(ScanError::NoMatch)
                }
            },
            Helper::ReadLine => parse_quote! {
                /// Reads a line from standard input into `buffer` as C's `fgets`
                /// does: up to the end of the line, which it keeps, or until
                /// `buffer` has room for one more byte, and a NUL after it.
                /// Returns whether there was anything to read.
                fn read_line(buffer: &mut [u8]) -> bool {
                    let mut length = 0;
                    while length + 1 < buffer.len() {
                        let Some(byte) = read_byte() else {
                            break;
                        };
                        buffer[length] = byte;
                        length += 1;
                        if byte == b'\n' {
                            break;
                        }
                    }
                    if length == 0 && buffer.len() != 1 {
                        return false;
                    }
                    buffer[length] = 0;
                    true
                }
            },
            Helper::ReadLineWithin => parse_quote! {
                /// `read_line` into the first `size` bytes of `buffer`, or into
                /// all of it where it has fewer: C's `fgets` is right with a
                /// size beyond its buffer as long as the line fits.
                fn read_line_within(buffer: &mut [u8], size: i32) -> bool {
                    let size = usize::try_from(size).unwrap_or(0);
                    let room = size.min(buffer.len());
                    // One byte holds only the NUL, which `fgets` stores at
                    // the end of the input for no size but one.
                    if room == 1 && size > 1 && peek_byte().is_none() {
                        return false;
                    }
                    read_line(&mut buffer[..room])
                }
            },
            Helper::UntilNul => parse_quote! {
                /// The text that C reads in `bytes`: the bytes before the first
                /// NUL, which must be there.
                fn until_nul(bytes: &[u8]) -> &[u8] {
                    let end = bytes
                        .iter()
                        .position(|&byte| byte == 0)
                        .expect("text ends with a NUL");
                    &bytes[..end]
                }
            },
            Helper::CopyText => parse_quote! {
                /// Copies the text in `source` to the start of `target` and
                /// fills the rest of `target` with NUL bytes, as C's `strncpy`
                /// does for the length of `target`. Text that is longer is cut
                /// short, without a NUL.
                fn copy_text(target: &mut [u8], source: &[u8]) {
                    let length = source
                        .iter()
                        .take(target.len())
                        .position(|&byte| byte == 0)
                        .unwrap_or(target.len());
                    target[..length].copy_from_slice(&source[..length]);
                    target[length..].fill(0);
                }
            },
            Helper::StoreText => parse_quote! {
                /// Stores `text` at the start of `target` with a NUL after it,
                /// as C's `sprintf` does, and returns its length. Text that
                /// does not fit stops the program, where C writes beyond
                /// `target`.
                fn store_text(text: &str, target: &mut [u8]) -> i32 {
                    let length = text.len();
                    target[..length].copy_from_slice(text.as_bytes());
                    target[length] = 0;
                    length as i32
                }
            },
            Helper::StoreTextCut => parse_quote! {
                /// Stores as much of `text` at the start of `target` as leaves
                /// room for a NUL after it, and the NUL, as C's `snprintf` does
                /// with the length of `target` as its size. Returns the length
                /// of all of `text`; an empty `target` gets nothing.
                fn store_text_cut(text: &str, target: &mut [u8]) -> i32 {
                    if let Some(room) = target.len().checked_sub(1) {
                        let length = text.len().min(room);
                        target[..length].copy_from_slice(&text.as_bytes()[..length]);
                        target[length] = 0;
                    }
                    text.len() as i32
                }
            },
            Helper::StoreTextWithin => parse_quote! {
                /// `store_text_cut` into the first `size` bytes of `target`, or
                /// into all of it where it has fewer: C's `snprintf` is right
                /// with a size beyond its buffer as long as the text fits, and
                /// writes beyond it otherwise, which stops the program here.
                fn store_text_within(text: &str, target: &mut [u8], size: u64) -> i32 {
                    let size = usize::try_from(size).unwrap_or(usize::MAX);
                    let room = size.min(target.len());
                    assert!(room == size || text.len() < room, "text beyond its buffer");
                    store_text_cut(text, &mut target[..room])
                }
            },
            Helper::CopyString => parse_quote! {
                /// Copies the text in `source` and the NUL that ends it to the
                /// start of `target`, as C's `strcpy` does.
                fn copy_string(target: &mut [u8], source: &[u8]) {
                    let text = until_nul(source);
                    target[..text.len()].copy_from_slice(text);
                    target[text.len()] = 0;
                }
            },
            Helper::AppendString => parse_quote! {
                /// Copies the text in `source` and the NUL that ends it to the
                /// end of the text in `target`, as C's `strcat` does.
                fn append_string(target: &mut [u8], source: &[u8]) {
                    let end = until_nul(target).len();
                    copy_string(&mut target[end..], source);
                }
            },
            Helper::AppendAtMost => parse_quote! {
                /// Copies at most `count` bytes of the text in `source` to the
                /// end of the text in `target`, and a NUL after them, as C's
                /// `strncat` does. The source needs no NUL within `count`
                /// bytes.
                fn append_at_most(target: &mut [u8], source: &[u8], count: usize) {
                    let end = until_nul(target).len();
                    let length = source
                        .iter()
                        .take(count)
                        .position(|&byte| byte == 0)
                        .unwrap_or(count);
                    target[end..][..length].copy_from_slice(&source[..length]);
                    target[end + length] = 0;
                }
            },
            Helper::TextWithin => parse_quote! {
                /// The text that C's `strncmp` reads in `bytes` within `count`
                /// bytes: those before the first NUL among them, or all `count`
                /// where there is none. Bytes that end before either are
                /// compared as far as they reach, where C reads beyond them
                /// unless the other text has differed by then.
                fn text_within(bytes: &[u8], count: usize) -> &[u8] {
                    let end = bytes
                        .iter()
                        .take(count)
                        .position(|&byte| byte == 0)
                        .unwrap_or(count.min(bytes.len()));
                    &bytes[..end]
                }
            },
            Helper::FirstBytes => parse_quote! {
                /// The first `count` bytes of `text`, or all of it where it is
                /// shorter: what C's `strncmp` reads of text that ends there.
                fn first_bytes(text: &[u8], count: usize) -> &[u8] {
                    &text[..count.min(text.len())]
                }
            },
            Helper::FindByte => parse_quote! {
                /// Where C's `strchr` finds `byte` in `text`, the bytes before
                /// the NUL, which it finds at the end.
                fn find_byte(text: &[u8], byte: u8) -> Option<usize> {
                    match byte {
                        0 => Some(text.len()),
                        _ => text.iter().position(|&found| found == byte),
                    }
                }
            },
            Helper::RfindByte => parse_quote! {
                /// Where C's `strrchr` finds the last `byte` in `text`, the
                /// bytes before the NUL, which it finds at the end.
                fn rfind_byte(text: &[u8], byte: u8) -> Option<usize> {
                    match byte {
                        0 => Some(text.len()),
                        _ => text.iter().rposition(|&found| found == byte),
                    }
                }
            },
            Helper::FindBytes => parse_quote! {
                /// Where C's `strstr` finds `part` in `text`, both without
                /// their NUL. An empty part is found at the start.
                fn find_bytes(text: &[u8], part: &[u8]) -> Option<usize> {
                    if part.is_empty() {
                        return Some(0);
                    }
                    text.windows(part.len()).position(|window| window == part)
                }
            },
            Helper::FindAny => parse_quote! {
                /// Where C's `strpbrk` finds one of the bytes of `set` in
                /// `text`, both without their NUL.
                fn find_any(text: &[u8], set: &[u8]) -> Option<usize> {
                    text.iter().position(|byte| set.contains(byte))
                }
            },
            Helper::TextOf => parse_quote! {
                /// The text that C reads in `bytes`, to print or to parse.
                fn text_of(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
                    String::from_utf8_lossy(until_nul(bytes))
                }
            },
        }
    }
}

/// What `break` leaves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Nest {
    Loop,
    /// One case of a `switch`, and whether a `break` in the middle of it
    /// leaves the `switch`.
    Switch {
        left: bool,
    },
}

/// The item `struct Globals`, and the value `main` starts it with.
type State = (syn::Item, syn::Expr);

struct Lower<'a> {
    program: &'a Program,
    facts: &'a Facts,
    names: Names,
    helpers: BTreeSet<Helper>,
    /// The enumerators the translation names.
    enumerators: BTreeSet<usize>,
    /// The function being translated, or none for a global's initialiser.
    function: Option<FuncId>,
    /// `main`'s parameters while `main` is translated.
    argc: Option<VarId>,
    argv: Option<VarId>,
    /// Whether the function being translated is a `main` that returns an
    /// exit code.
    exits: bool,
    nesting: Vec<Nest>,
}

/// Translates the program.
pub fn program(program: &Program, facts: &Facts) -> R<syn::File> {
    let mut lower = Lower {
        program,
        facts,
        names: Names::new(program, facts),
        helpers: BTreeSet::new(),
        enumerators: BTreeSet::new(),
        function: None,
        argc: None,
        argv: None,
        exits: false,
        nesting: Vec::new(),
    };
    let mut records = Vec::new();
    for (id, record) in program.records.iter().enumerate() {
        if record.own {
            records.extend(lower.record(id)?);
        }
    }
    let (constants, state) = lower.globals()?;
    let mut functions = Vec::new();
    let mut uses_exit_code = false;
    for (id, function) in program.functions.iter().enumerate() {
        functions.push(if function.name == "main" {
            let main = lower.main(id, state.as_ref().map(|(_, init)| init))?;
            uses_exit_code = lower.exits;
            main
        } else {
            lower.function(id)?
        });
    }
    if program.function("main").is_none() {
        return unsupported("a program without `main`", 0);
    }
    let mut items: Vec<syn::Item> = Vec::new();
    if lower.helpers.iter().any(|helper| helper.reads_input()) {
        items.push(parse_quote!(
            use std::io::BufRead;
        ));
    }
    if uses_exit_code {
        items.push(parse_quote!(
            use std::process::ExitCode;
        ));
    }
    for &index in &lower.enumerators {
        let enumerator = &program.enumerators[index];
        let (name, value) = (
            rust::ident(lower.names.enumerator(index)),
            rust::int(enumerator.value, ""),
        );
        items.push(parse_quote!(const #name: i32 = #value;));
    }
    items.extend(constants);
    items.extend(records);
    items.extend(state.map(|(item, _)| item));
    items.extend(functions);
    items.extend(lower.helpers.iter().map(|helper| helper.item()));
    Ok(syn::File {
        shebang: None,
        attrs: Vec::new(),
        items,
    })
}

impl Lower<'_> {
    /// Notes that the translation calls the helper, with those it calls.
    fn helper(&mut self, helper: Helper) {
        if self.helpers.insert(helper) {
            for &needed in helper.needs() {
                self.helper(needed);
            }
        }
    }

    /// R5: a `struct` for each record, with the traits its uses need.
    fn record(&mut self, id: usize) -> R<Vec<syn::Item>> {
        let record = &self.program.records[id];
        // `normalise::unions` has left no use of a union that is the bits
        // of a number.
        if self.program.bits_of_union(id).is_some() {
            return Ok(Vec::new());
        }
        if record.union {
            return unsupported(
                "a union whose members are not all numbers of one size: no rule yet",
                0,
            );
        }
        let name = rust::ident(self.names.record(id));
        let mut fields: Vec<syn::Field> = Vec::new();
        // A record with a pointer to a function that holds no NULL has
        // no zero, and `zero` refuses where one would be needed.
        let mut zeros: Option<Vec<syn::FieldValue>> = Some(Vec::new());
        for (index, field) in record.fields.iter().enumerate() {
            let slot = Slot::Field(id, index);
            let ty = self.field_type(slot, &field.ty)?;
            let zero = match self.zero_of(&field.ty, Some(slot), 0) {
                Ok(zero) => Some(zero),
                Err(_) if field.ty.function().is_some() => None,
                Err(error) => return Err(error),
            };
            let name = self.names.field(id, index);
            let field = rust::ident(name);
            fields.push(
                syn::parse::Parser::parse2(syn::Field::parse_named, quote::quote!(#field: #ty))
                    .expect("a field"),
            );
            zeros = zeros.zip(zero).map(|(mut zeros, zero)| {
                zeros.push(rust::field_value(name, zero));
                zeros
            });
        }
        let ty = Type::Record(id);
        let copy = self.is_copy(&ty).then(|| quote::quote!(Copy,));
        let derived = self.derives_default(&ty);
        let default = derived.then(|| quote::quote!(Default,));
        let mut items: Vec<syn::Item> = vec![parse_quote! {
            #[derive(Clone, #copy #default)]
            struct #name { #(#fields),* }
        }];
        if let (false, Some(zeros)) = (derived, zeros) {
            items.push(parse_quote! {
                impl Default for #name {
                    fn default() -> Self {
                        Self { #(#zeros),* }
                    }
                }
            });
        }
        Ok(items)
    }

    /// The type of a record's field or of a global, which holds its value
    /// or owns what it points to.
    fn field_type(&mut self, slot: Slot, ty: &Type) -> R<syn::Type> {
        if !ty.is_pointer() {
            return self.ty(ty, 0);
        }
        if ty.function().is_none() && !self.repr(slot, 0)?.owned {
            return unsupported(
                format!("{} borrows what it points to", self.slot_name(slot)),
                0,
            );
        }
        self.slot_type(slot, ty, None, 0)
    }

    /// R9: a global that never changes is a constant; the others are the
    /// fields of `Globals`, which `main` creates and the functions that use
    /// them take as their first parameter. Returns the constants, and
    /// `Globals` with its first value.
    fn globals(&mut self) -> R<(Vec<syn::Item>, Option<State>)> {
        let program = self.program;
        let mut constants: Vec<syn::Item> = Vec::new();
        let mut fields: Vec<syn::Field> = Vec::new();
        let mut values: Vec<syn::FieldValue> = Vec::new();
        for &global in &program.globals {
            let var = &program.vars[global];
            let name = rust::ident(self.names.var(global));
            let slot = var.ty.is_pointer().then_some(Slot::Var(global));
            let value = match &var.init {
                Some(init) => self.init(&var.ty, slot, init)?,
                None => self.zero_of(&var.ty, slot, var.at)?,
            };
            if self.facts.mutated(global) {
                let ty = self.field_type(Slot::Var(global), &var.ty)?;
                fields.push(
                    syn::parse::Parser::parse2(syn::Field::parse_named, quote::quote!(#name: #ty))
                        .expect("a field"),
                );
                values.push(rust::field_value(self.names.var(global), value));
            } else if var.ty.function().is_some() {
                let ty = self.slot_type(Slot::Var(global), &var.ty, None, var.at)?;
                constants.push(parse_quote!(const #name: #ty = #value;));
            } else if var.ty.is_pointer() {
                return unsupported(
                    format!("the global pointer `{}` that never changes", var.name),
                    var.at,
                );
            } else {
                let ty = self.ty(&var.ty, var.at)?;
                constants.push(match var.ty {
                    Type::Array(..) | Type::Record(_) => parse_quote!(static #name: #ty = #value;),
                    _ => parse_quote!(const #name: #ty = #value;),
                });
            }
        }
        if fields.is_empty() {
            return Ok((constants, None));
        }
        let item = parse_quote! {
            /// The program's global variables.
            struct Globals { #(#fields),* }
        };
        Ok((
            constants,
            Some((item, parse_quote!(Globals { #(#values),* }))),
        ))
    }

    /// Whether a local or a parameter needs `mut`.
    fn needs_mut(&self, var: VarId) -> bool {
        let pointer = self.facts.pointer(Slot::Var(var));
        // An owner and an `Option` are written through their binding.
        self.facts.mutated(var)
            || (self.facts.stored_through(var) && (pointer.owned || pointer.nullable))
    }

    /// R1: a function, with a borrow for each pointer it does not own.
    fn function(&mut self, id: FuncId) -> R<syn::Item> {
        let program = self.program;
        let function = &program.functions[id];
        if function.variadic {
            return unsupported(format!("the variadic function `{}`", function.name), 0);
        }
        self.function = Some(id);
        self.exits = false;
        let access = self.facts.access(id);
        // A pointer to a function borrows nothing.
        let borrows = |lower: &Self, ty: &Type, slot: Slot| match ty.function() {
            Some(_) => Ok(false),
            None => lower.repr(slot, 0).map(|repr| !repr.owned),
        };
        let mut borrowed = usize::from(access != Access::None);
        for &param in &function.params {
            let ty = &program.vars[param].ty;
            if ty.is_pointer() && borrows(self, ty, Slot::Var(param))? {
                borrowed += 1;
            }
        }
        // A borrowed result with one borrowed parameter needs no lifetime.
        // With none it can only point to a constant, and with several the
        // translation does not know which, so all share one.
        let returns_borrow =
            function.ret.is_pointer() && borrows(self, &function.ret, Slot::Return(id))?;
        let (shared, result): (Option<syn::Lifetime>, Option<syn::Lifetime>) =
            match (returns_borrow, borrowed) {
                (false, _) | (true, 1) => (None, None),
                (true, 0) => (None, Some(parse_quote!('static))),
                (true, _) => (Some(parse_quote!('a)), Some(parse_quote!('a))),
            };
        let mut params: Vec<syn::FnArg> = Vec::new();
        match access {
            Access::None => {}
            Access::Read => params.push(parse_quote!(globals: & #shared Globals)),
            Access::Write => params.push(parse_quote!(globals: & #shared mut Globals)),
        }
        for &param in &function.params {
            let var = &program.vars[param];
            let name = rust::ident(self.names.var(param));
            let mutability: Option<syn::Token![mut]> = self.needs_mut(param).then(Default::default);
            let ty = match var.ty.is_pointer() {
                true => self.slot_type(Slot::Var(param), &var.ty, shared.as_ref(), var.at)?,
                false => self.ty(&var.ty, var.at)?,
            };
            params.push(parse_quote!(#mutability #name: #ty));
        }
        let ret = match &function.ret {
            Type::Void => None,
            ty if ty.is_pointer() => {
                Some(self.slot_type(Slot::Return(id), ty, result.as_ref(), 0)?)
            }
            ty => Some(self.ty(ty, 0)?),
        };
        let body = self.body(&function.body)?;
        let name = rust::ident(self.names.function(id));
        let (generics, ret) = (shared.iter(), ret.iter());
        Ok(parse_quote! {
            fn #name #(<#generics>)* (#(#params),*) #(-> #ret)* {
                #(#body)*
            }
        })
    }

    /// R1: `main`, which returns an exit code unless it only ever returns
    /// 0, and reads its arguments from `args`.
    fn main(&mut self, id: FuncId, state: Option<&syn::Expr>) -> R<syn::Item> {
        let program = self.program;
        let function = &program.functions[id];
        self.function = Some(id);
        (self.argc, self.argv) = match function.params[..] {
            [] => (None, None),
            [argc, argv] => (Some(argc), Some(argv)),
            _ => return unsupported("a `main` with other parameters", 0),
        };
        let mut body: Vec<syn::Stmt> = Vec::new();
        let named = |var: Option<VarId>| {
            any_expr(
                &function.body,
                &program.vars,
                &mut |expr| matches!(expr.kind, ExprKind::Var(named) if Some(named) == var),
            )
        };
        if named(self.argc) || named(self.argv) {
            body.push(parse_quote!(let args: Vec<String> = std::env::args().collect();));
        }
        if let Some(state) = state {
            body.push(match self.facts.access(id) {
                Access::None => parse_quote!(let _globals = #state;),
                Access::Read => parse_quote!(let globals = &#state;),
                Access::Write => parse_quote!(let globals = &mut #state;),
            });
        }
        self.exits = !returns_only_zero(&function.body, program, self.facts);
        body.extend(self.body(&function.body)?);
        if self.exits && !matches!(function.body.last(), Some(Stmt::Return(Some(_)))) {
            body.push(syn::Stmt::Expr(parse_quote!(ExitCode::SUCCESS), None));
        }
        (self.argc, self.argv) = (None, None);
        let ret = self.exits.then(|| quote::quote!(-> ExitCode));
        Ok(parse_quote! {
            fn main() #ret {
                #(#body)*
            }
        })
    }

    /// The statements of a function, whose last `return` is its value or,
    /// with no value, needs no code.
    fn body(&mut self, stmts: &[Stmt]) -> R<Vec<syn::Stmt>> {
        let (rest, last) = match stmts.split_last() {
            Some((Stmt::Return(Some(value)), rest)) => (rest, Some(value)),
            Some((Stmt::Return(None), rest)) => (rest, None),
            _ => (stmts, None),
        };
        let mut body = self.stmts(rest)?;
        if let Some(value) = last {
            if !self.in_main() {
                body.push(syn::Stmt::Expr(self.returned(value)?, None));
            } else if self.exits {
                body.push(syn::Stmt::Expr(self.exit_code(value)?, None));
            }
            // A `main` without an exit code ends with `return 0`, which
            // needs no code.
        }
        Ok(body)
    }

    /// Whether the function being translated is `main`.
    fn in_main(&self) -> bool {
        self.function
            .is_some_and(|id| self.program.functions[id].name == "main")
    }

    /// The value a function returns for `return value`.
    fn returned(&mut self, value: &Expr) -> R<syn::Expr> {
        match (self.function, value.ty.is_pointer()) {
            (Some(function), true) => {
                let repr = self.repr(Slot::Return(function), value.at)?;
                self.pointer(value, repr)
            }
            _ => self.typed(value),
        }
    }

    /// The exit code of `main` for `return value`.
    fn exit_code(&mut self, value: &Expr) -> R<syn::Expr> {
        Ok(match value.constant(self.program) {
            Some(0) => parse_quote!(ExitCode::SUCCESS),
            // The system keeps the low byte.
            Some(code) => rust::call("ExitCode::from", [rust::int(code & 0xff, "")]),
            None => {
                let code = rust::cast(self.expr(value)?, parse_quote!(u8));
                rust::call("ExitCode::from", [code])
            }
        })
    }
}

/// Whether every `return` that `main` reaches returns the constant 0.
fn returns_only_zero(stmts: &[Stmt], program: &Program, facts: &Facts) -> bool {
    let only_zero = |stmts: &[Stmt]| returns_only_zero(stmts, program, facts);
    stmts.iter().all(|stmt| match stmt {
        Stmt::Return(Some(value)) => value.constant(program) == Some(0),
        Stmt::Return(None) => false,
        Stmt::Block(body)
        | Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::Switch(_, body)
        | Stmt::For { body, .. } => only_zero(body),
        // A branch that the facts rule out returns nothing.
        Stmt::If(cond, then, other) => match facts.truth(program, cond) {
            Some(true) => only_zero(then),
            Some(false) => only_zero(other),
            None => only_zero(then) && only_zero(other),
        },
        Stmt::IfSome { then, other, .. } => only_zero(then) && only_zero(other),
        Stmt::LetSome { otherwise, .. } => only_zero(otherwise),
        _ => true,
    })
}
