//! C's formatted input and output: `printf` becomes `print!` with the format
//! converted when the program is translated, and `scanf` calls a small
//! runtime that reads standard input as glibc does.

use crate::translate::{Translator, c_type, inner, kind, unsupported};
use serde_json::Value;

/// `printf` with a literal format, whose result the program discards.
pub(crate) fn printf(t: &mut Translator, args: &[Value]) -> String {
    let Some(format) = args.first().and_then(string_literal) else {
        return unsupported("printf without a literal format");
    };
    let (mut format, types) = match printf_format(&format) {
        Ok(converted) => converted,
        Err(err) => return unsupported(&err),
    };
    let values = &args[1..];
    if values.len() != types.len() {
        return unsupported("printf with the wrong number of arguments");
    }
    let mut call = String::new();
    for (value, ty) in values.iter().zip(types) {
        if c_type(value) != ty {
            return unsupported(&format!("printf of {} as {ty}", c_type(value)));
        }
        call.push_str(", ");
        call.push_str(&t.expr(value));
    }
    let mac = if format.ends_with('\n') {
        format.pop();
        "println"
    } else {
        "print"
    };
    format!("{mac}!({format:?}{call})")
}

/// Converts a printf format to a Rust one and lists the C types of the
/// arguments it reads.
fn printf_format(format: &[u8]) -> Result<(String, Vec<&'static str>), String> {
    let Ok(format) = std::str::from_utf8(format) else {
        return Err("printf format that is not UTF-8".to_string());
    };
    let mut out = String::new();
    let mut types = Vec::new();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            '%' => {
                let (mut left, mut plus, mut zero) = (false, false, false);
                while let Some(&flag @ ('-' | '+' | '0')) = chars.peek() {
                    chars.next();
                    match flag {
                        '-' => left = true,
                        '+' => plus = true,
                        _ => zero = true,
                    }
                }
                let mut width = String::new();
                while let Some(&digit @ '0'..='9') = chars.peek() {
                    chars.next();
                    width.push(digit);
                }
                match chars.next() {
                    Some('%') if !(left || plus || zero) && width.is_empty() => out.push('%'),
                    Some('d' | 'i') => {
                        // Rust right-aligns numbers already; C ignores 0 with -.
                        let spec = format!(
                            "{}{}{}{width}",
                            if left { "<" } else { "" },
                            if plus { "+" } else { "" },
                            if zero && !left { "0" } else { "" },
                        );
                        if spec.is_empty() {
                            out.push_str("{}");
                        } else {
                            out.push_str(&format!("{{:{spec}}}"));
                        }
                        types.push("int");
                    }
                    other => {
                        return Err(format!(
                            "printf conversion %{}",
                            other.map_or(String::new(), String::from)
                        ));
                    }
                }
            }
            c => out.push(c),
        }
    }
    Ok((out, types))
}

/// `scanf` with a literal format, as an `i32` expression.
pub(crate) fn scanf(t: &mut Translator, args: &[Value]) -> String {
    let Some(format) = args.first().and_then(string_literal) else {
        return unsupported("scanf without a literal format");
    };
    let types = match scanf_format(&format) {
        Ok(types) => types,
        Err(err) => return unsupported(&err),
    };
    let targets = &args[1..];
    if targets.len() != types.len() {
        return unsupported("scanf with the wrong number of arguments");
    }
    // One closure stores every value, so a variable named twice, as in
    // `scanf("%d%d", &x, &x)`, is borrowed once and written in order. Its
    // parameters have names that C reserves, so no C variable hides them.
    let mut arms = String::new();
    for (i, (target, ty)) in targets.iter().zip(types).enumerate() {
        if kind(target) != "UnaryOperator" || target["opcode"] != "&" {
            return unsupported("scanf into a pointer variable");
        }
        let place = &inner(target)[0];
        if c_type(place) != ty {
            return unsupported(&format!("scanf of {ty} into {}", c_type(place)));
        }
        arms.push_str(&format!("{i} => {} = __value as i32, ", t.place(place)));
    }
    t.uses_scanf = true;
    format!(
        "crt::scanf({}, |__arg, __value| match __arg {{ {arms}_ => {{}} }})",
        byte_string(&format),
    )
}

/// Lists the C types that a scanf format stores into.
fn scanf_format(format: &[u8]) -> Result<Vec<&'static str>, String> {
    let mut types = Vec::new();
    let mut bytes = format.iter();
    while let Some(&byte) = bytes.next() {
        if byte == b'%' {
            match bytes.next() {
                Some(b'%') => {}
                Some(b'd') => types.push("int"),
                other => {
                    return Err(format!(
                        "scanf conversion %{}",
                        other.map_or(String::new(), |&b| char::from(b).to_string())
                    ));
                }
            }
        }
    }
    Ok(types)
}

/// The bytes of a narrow string literal, without its terminating zero.
fn string_literal(expr: &Value) -> Option<Vec<u8>> {
    let mut expr = expr;
    while matches!(kind(expr), "ImplicitCastExpr" | "ParenExpr") {
        expr = inner(expr).first()?;
    }
    if kind(expr) != "StringLiteral" {
        return None;
    }
    unescape(expr["value"].as_str()?)
}

/// Decodes a string literal as Clang prints it: in double quotes, with C's
/// escapes. Clang has already joined adjacent literals.
fn unescape(literal: &str) -> Option<Vec<u8>> {
    let body = literal.strip_prefix('"')?.strip_suffix('"')?.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if body[i] != b'\\' {
            out.push(body[i]);
            i += 1;
            continue;
        }
        let escape = *body.get(i + 1)?;
        i += 2;
        out.push(match escape {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'v' => 0x0b,
            b'e' => 0x1b,
            b'0'..=b'7' => {
                let mut value = u32::from(escape - b'0');
                for _ in 0..2 {
                    match body.get(i) {
                        Some(&d @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(d - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                u8::try_from(value).ok()?
            }
            b'x' => {
                let start = i;
                while body.get(i).is_some_and(u8::is_ascii_hexdigit) {
                    i += 1;
                }
                let digits = std::str::from_utf8(&body[start..i]).ok()?;
                u8::from_str_radix(digits, 16).ok()?
            }
            other => other,
        });
    }
    Some(out)
}

/// A Rust byte string literal holding `bytes`.
fn byte_string(bytes: &[u8]) -> String {
    let mut out = String::from("b\"");
    for &byte in bytes {
        out.extend(std::ascii::escape_default(byte).map(char::from));
    }
    out.push('"');
    out
}

/// Emitted once into a translation that calls `scanf`.
pub(crate) const SCANF_RUNTIME: &str = r#"/// C library functions that Rust's standard library does not match.
mod crt {
    use std::io::BufRead;

    /// C's `isspace` in the C locale.
    fn is_space(byte: u8) -> bool {
        matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
    }

    fn peek(input: &mut impl BufRead) -> Option<u8> {
        input.fill_buf().ok()?.first().copied()
    }

    fn skip_space(input: &mut impl BufRead) {
        while peek(input).is_some_and(is_space) {
            input.consume(1);
        }
    }

    /// Reads a decimal integer as glibc does: a sign it reads stays read,
    /// and a value out of range saturates as with `strtol`.
    fn read_int(input: &mut impl BufRead) -> Option<i64> {
        let negative = match peek(input) {
            Some(sign @ (b'+' | b'-')) => {
                input.consume(1);
                sign == b'-'
            }
            _ => false,
        };
        // Counts down, since i64::MIN has no positive counterpart.
        let mut value: i64 = 0;
        let mut digits = false;
        while let Some(digit @ b'0'..=b'9') = peek(input) {
            input.consume(1);
            digits = true;
            value = value
                .saturating_mul(10)
                .saturating_sub(i64::from(digit - b'0'));
        }
        if !digits {
            return None;
        }
        Some(if negative {
            value
        } else {
            value.checked_neg().unwrap_or(i64::MAX)
        })
    }

    /// Reads standard input as C's `scanf` does, passing each converted
    /// value to `store` with the index of its argument. Returns the number
    /// of values stored, or -1 if input ended before the first.
    pub fn scanf(format: &[u8], mut store: impl FnMut(usize, i64)) -> i32 {
        let mut input = std::io::stdin().lock();
        let mut stored = 0;
        let ended = |stored| if stored == 0 { -1 } else { stored };
        let mut format = format.iter();
        while let Some(&byte) = format.next() {
            if is_space(byte) {
                skip_space(&mut input);
                continue;
            }
            if byte == b'%' {
                if format.next() != Some(&b'%') {
                    skip_space(&mut input);
                    if peek(&mut input).is_none() {
                        return ended(stored);
                    }
                    let Some(value) = read_int(&mut input) else {
                        return stored;
                    };
                    store(stored as usize, value);
                    stored += 1;
                    continue;
                }
                skip_space(&mut input);
            }
            match peek(&mut input) {
                None => return ended(stored),
                Some(next) if next == byte => input.consume(1),
                Some(_) => return stored,
            }
        }
        stored
    }
}
"#;
