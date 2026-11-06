//! C's formatted input and output: `printf` becomes `print!` with the format
//! converted when the program is translated, and `scanf` and `fgets` call a
//! small runtime that reads standard input as glibc does.

use crate::translate::{Translator, c_type, inner, kind, rust_type, unsupported};
use serde_json::Value;

/// `printf` with a literal format, whose result the program discards, as
/// the Rust macro `mac`, `print` or `eprint`.
pub(crate) fn printf(t: &mut Translator, mac: &str, args: &[Value]) -> String {
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
        call.push_str(", ");
        match (c_type(value), ty) {
            ("double", Argument::Float(conversion, precision)) => {
                t.require(FLOAT_RUNTIME);
                call.push_str(&format!(
                    "crt::float({}, b'{conversion}', {precision})",
                    t.expr(value)
                ));
            }
            (arg, Argument::Float(..)) => {
                return unsupported(&format!("printf of {arg} as double"));
            }
            ("char *", Argument::Value("char *")) => {
                t.require(C_STR_RUNTIME);
                call.push_str(&format!("crt::c_str({})", t.expr(value)));
            }
            (arg, Argument::Value(ty)) if arg == ty => call.push_str(&t.expr(value)),
            // glibc reads the bits of an int as unsigned, as Rust's `as` does.
            ("int", Argument::Value("unsigned int")) => {
                call.push_str(&format!("{} as u32", t.operand(value)));
            }
            (arg, Argument::Value(ty)) => {
                return unsupported(&format!("printf of {arg} as {ty}"));
            }
        }
    }
    let line = if format.ends_with('\n') {
        format.pop();
        "ln"
    } else {
        ""
    };
    format!("{mac}{line}!({format:?}{call})")
}

/// What a printf conversion reads.
enum Argument {
    /// A value of this C type, which Rust formats itself.
    Value(&'static str),
    /// A `double` that `crt::float` formats with this conversion, such as
    /// `f`, and this precision.
    Float(char, usize),
}

/// Converts a printf format to a Rust one and lists the arguments it reads.
fn printf_format(format: &[u8]) -> Result<(String, Vec<Argument>), String> {
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
                // A point without digits is the precision 0.
                let mut precision = None;
                if chars.next_if_eq(&'.').is_some() {
                    let mut digits = String::new();
                    while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                        digits.push(digit);
                    }
                    precision = Some(digits.parse().unwrap_or(0));
                }
                // `l` changes nothing for a floating conversion.
                let long = chars.next_if_eq(&'l').is_some();
                match chars.next() {
                    // Rust pads strings on the right by default.
                    Some(conversion @ ('f' | 'F' | 'e' | 'E' | 'g' | 'G')) if !(plus || zero) => {
                        if width.is_empty() {
                            out.push_str("{}");
                        } else {
                            out.push_str(&format!("{{:{}{width}}}", if left { "<" } else { ">" }));
                        }
                        types.push(Argument::Float(conversion, precision.unwrap_or(6)));
                    }
                    Some('f' | 'F' | 'e' | 'E' | 'g' | 'G') => {
                        return Err("printf floating conversion with the flag + or 0".into());
                    }
                    _ if precision.is_some() || long => {
                        return Err(
                            "printf precision or length on a conversion that is not floating"
                                .into(),
                        );
                    }
                    Some('%') if !(left || plus || zero) && width.is_empty() => out.push('%'),
                    // A width counts bytes in C but characters in Rust.
                    Some('s') if !(left || plus || zero) && width.is_empty() => {
                        out.push_str("{}");
                        types.push(Argument::Value("char *"));
                    }
                    Some('s') => return Err("printf conversion %s with flags or a width".into()),
                    Some(conversion @ ('d' | 'i' | 'u' | 'o' | 'x' | 'X')) => {
                        let (ty, radix) = match conversion {
                            'd' | 'i' => ("int", ""),
                            'u' => ("unsigned int", ""),
                            'o' => ("unsigned int", "o"),
                            'x' => ("unsigned int", "x"),
                            _ => ("unsigned int", "X"),
                        };
                        // Rust right-aligns numbers already; C ignores 0 with
                        // -, and + on unsigned conversions.
                        let spec = format!(
                            "{}{}{}{width}{radix}",
                            if left { "<" } else { "" },
                            if plus && ty == "int" { "+" } else { "" },
                            if zero && !left { "0" } else { "" },
                        );
                        if spec.is_empty() {
                            out.push_str("{}");
                        } else {
                            out.push_str(&format!("{{:{spec}}}"));
                        }
                        types.push(Argument::Value(ty));
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
        let rust = rust_type(ty).unwrap_or_default();
        let store = t.store(place, &format!("__value as {rust}"));
        arms.push_str(&format!("{i} => {store}, "));
    }
    t.require(SCANF_RUNTIME);
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
                Some(b'u') => types.push("unsigned int"),
                Some(b'c') => types.push("char"),
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

/// The name of the standard stream that `expr` names, such as `stdin`.
pub(crate) fn stream(expr: &Value) -> Option<&str> {
    let mut expr = expr;
    while matches!(kind(expr), "ImplicitCastExpr" | "ParenExpr") {
        expr = inner(expr).first()?;
    }
    match expr["referencedDecl"]["name"].as_str() {
        Some(name @ ("stdin" | "stdout" | "stderr")) if kind(expr) == "DeclRefExpr" => Some(name),
        _ => None,
    }
}

/// The bytes of a narrow string literal up to its first zero, where C's
/// library functions stop reading it, as with the format `"a\0b"`.
fn string_literal(expr: &Value) -> Option<Vec<u8>> {
    let mut expr = expr;
    while kind(expr) == "ImplicitCastExpr" {
        expr = inner(expr).first()?;
    }
    let mut bytes = string_array(expr)?;
    if let Some(end) = bytes.iter().position(|&byte| byte == 0) {
        bytes.truncate(end);
    }
    Some(bytes)
}

/// The bytes of the array that a narrow string literal makes, through the
/// zero that C adds at its end.
pub(crate) fn string_array(expr: &Value) -> Option<Vec<u8>> {
    let mut expr = expr;
    while kind(expr) == "ParenExpr" {
        expr = inner(expr).first()?;
    }
    if kind(expr) != "StringLiteral" || !c_type(expr).starts_with("char[") {
        return None;
    }
    let mut bytes = unescape(expr["value"].as_str()?)?;
    bytes.push(0);
    Some(bytes)
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
pub(crate) fn byte_string(bytes: &[u8]) -> String {
    let mut out = String::from("b\"");
    for &byte in bytes {
        out.extend(std::ascii::escape_default(byte).map(char::from));
    }
    out.push('"');
    out
}

/// The part of the `crt` module that `printf`'s `%s` calls. The strings come
/// from literals, which the translation takes only when they are UTF-8.
pub(crate) const C_STR_RUNTIME: &str = r#"    /// The text of a C string, its bytes before the zero that ends it. C
    /// has no string at a null pointer.
    pub fn c_str(s: Option<&[u8]>) -> &str {
        let s = s.expect("null pointer as a C string");
        let end = s.iter().position(|&byte| byte == 0).unwrap_or(s.len());
        std::str::from_utf8(&s[..end]).expect("C string that is not UTF-8")
    }
"#;

/// The part of the `crt` module that `printf`'s floating conversions call.
pub(crate) const FLOAT_RUNTIME: &str = r#"    /// `printf`'s `%f`, `%e` or `%g` of `value` with `precision` as glibc
    /// formats it, or `%F`, `%E` or `%G` in capitals. Rust rounds the exact
    /// value to the nearest decimal, ties to even, as glibc does.
    pub fn float(value: f64, conversion: u8, precision: usize) -> String {
        let text = if value.is_nan() {
            String::from(if value.is_sign_negative() { "-nan" } else { "nan" })
        } else if value.is_infinite() {
            String::from(if value < 0.0 { "-inf" } else { "inf" })
        } else {
            match conversion.to_ascii_lowercase() {
                b'f' => format!("{value:.precision$}"),
                b'e' => {
                    let e = format!("{value:.precision$e}");
                    let (mantissa, power) = e.split_once('e').unwrap();
                    exponent(mantissa, power.parse().unwrap())
                }
                // `%e`'s exponent with `precision` significant digits picks
                // `%f` from -4 to below `precision`, and trailing zeros go.
                _ => {
                    let digits = precision.max(1) as i64;
                    let e = format!("{value:.*e}", digits as usize - 1);
                    let (mantissa, power) = e.split_once('e').unwrap();
                    let power: i64 = power.parse().unwrap();
                    if (-4..digits).contains(&power) {
                        let fixed = format!("{value:.*}", (digits - 1 - power) as usize);
                        trim(&fixed).to_string()
                    } else {
                        exponent(trim(mantissa), power)
                    }
                }
            }
        };
        if conversion.is_ascii_uppercase() {
            text.to_ascii_uppercase()
        } else {
            text
        }
    }

    /// C's exponent after `mantissa`: a sign and at least two digits.
    fn exponent(mantissa: &str, power: i64) -> String {
        let sign = if power < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", power.unsigned_abs())
    }

    /// A number without the zeros that end its fraction, nor its point if
    /// they are all of it, as `%g` prints it.
    fn trim(number: &str) -> &str {
        if number.contains('.') {
            number.trim_end_matches('0').trim_end_matches('.')
        } else {
            number
        }
    }
"#;

/// The part of the `crt` module that `scanf` calls.
pub(crate) const SCANF_RUNTIME: &str = r#"    use std::io::BufRead;

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
    /// and a value out of range saturates as with `strtol`, or for
    /// `unsigned`, as with `strtoul`, which negates a value after `-` modulo
    /// 2^64; that value is given as the bits of an `i64`.
    fn read_int(input: &mut impl BufRead, unsigned: bool) -> Option<i64> {
        let negative = match peek(input) {
            Some(sign @ (b'+' | b'-')) => {
                input.consume(1);
                sign == b'-'
            }
            _ => false,
        };
        let mut magnitude = Some(0u64);
        let mut digits = false;
        while let Some(digit @ b'0'..=b'9') = peek(input) {
            input.consume(1);
            digits = true;
            magnitude = magnitude
                .and_then(|m| m.checked_mul(10))
                .and_then(|m| m.checked_add(u64::from(digit - b'0')));
        }
        if !digits {
            return None;
        }
        Some(match magnitude {
            Some(m) if unsigned && negative => m.wrapping_neg() as i64,
            Some(m) if unsigned => m as i64,
            None if unsigned => u64::MAX as i64,
            Some(m) if negative => 0i64.checked_sub_unsigned(m).unwrap_or(i64::MIN),
            Some(m) => i64::try_from(m).unwrap_or(i64::MAX),
            None if negative => i64::MIN,
            None => i64::MAX,
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
                let conversion = format.next();
                if conversion != Some(&b'%') {
                    // `%c` takes the next byte, even a space.
                    if conversion != Some(&b'c') {
                        skip_space(&mut input);
                    }
                    let value = match peek(&mut input) {
                        None => return ended(stored),
                        Some(byte) if conversion == Some(&b'c') => {
                            input.consume(1);
                            i64::from(byte)
                        }
                        Some(_) => match read_int(&mut input, conversion == Some(&b'u')) {
                            Some(value) => value,
                            None => return stored,
                        },
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
"#;

/// The part of the `crt` module that `fgets` calls.
pub(crate) const FGETS_RUNTIME: &str = r#"    /// C's `fgets` on standard input as glibc has it: reads bytes into `s`
    /// up to and including a newline, at most `n - 1`, and a zero after
    /// them. Returns None, leaving `s` as it was, when `n` is not positive
    /// or input ends or fails before a byte; `n` of 1 reads nothing and
    /// stores the zero. Writing past the array panics.
    pub fn fgets(s: &mut [u8], n: i32) -> Option<()> {
        use std::io::BufRead;
        let room = usize::try_from(n).ok().filter(|&n| n > 0)? - 1;
        let mut input = std::io::stdin().lock();
        let mut read = 0;
        while read < room {
            let Some(&byte) = input.fill_buf().ok()?.first() else {
                break;
            };
            input.consume(1);
            s[read] = byte;
            read += 1;
            if byte == b'\n' {
                break;
            }
        }
        if read == 0 && room > 0 {
            return None;
        }
        s[read] = 0;
        Some(())
    }
"#;
