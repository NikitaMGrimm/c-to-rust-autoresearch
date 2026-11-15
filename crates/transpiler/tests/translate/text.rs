use crate::common::{assert_lines, check, translate};

#[test]
fn a_pointer_that_only_holds_text_is_a_str() {
    let rust = check(
        "text",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
const char *name(int kind) {
    switch (kind) {
        case 0: return "zero";
        case 1: return "one\0hidden";
        default: return "many {braces}";
    }
}
void line(const char *text) {
    if (text != NULL) {
        printf("%s: %lu\n", text, strlen(text));
    }
}
void say(const char *text) { puts(text); }
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int kind = atoi(argv[1]);
    const char *parity = kind % 2 ? "odd" : "even";
    const char *nothing;
    line(name(kind));
    line(parity);
    line(nothing);
    say(parity);
    say("12 %d");
    printf("%d %s\n", atoi("42") + (int)strlen("four"), name(kind + 1));
    return 0;
}
"#,
        &[&["0"], &["1"], &["2"], &["7"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn name(kind: i32) -> &'static str {",
            "return \"one\";",
            "fn line(text: Option<&str>) {",
            "if let Some(text) = text {",
            "println!(\"{}: {}\", text, text.len() as u64);",
            "fn say(text: &str) {",
            "println!(\"{}\", text);",
            "let parity: &str = if kind % 2 != 0 { \"odd\" } else { \"even\" };",
            "let nothing: Option<&str> = None;",
            "line(Some(name(kind)));",
            "line(nothing);",
        ],
    );
}

#[test]
fn text_with_no_rule_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "text-width",
            "#include <stdio.h>\nint main(void) {\n    const char *text = \"abc\";\n    printf(\"%5s\", text);\n    return 0;\n}\n"
        ),
        "unsupported: a `printf` width for text in a variable, at line 4"
    );
}

#[test]
fn an_end_with_no_one_call_before_its_read_is_refused() {
    let refused = |name, body: &str| {
        let c = format!(
            "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char **argv) {{\n    if (argc < 3) return 2;\n{body}    return 0;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    let no_call = "unsupported: an end pointer other than a local `char *` that one call sets before each read: no rule yet, at line ";
    // Either call may have set the end that is read.
    assert_eq!(
        refused(
            "end-branches",
            "    char *end;\n    long n;\n    if (argc > 3) n = strtol(argv[1], &end, 10);\n    else n = strtol(argv[2], &end, 10);\n    printf(\"%ld %d\\n\", n, *end);\n"
        ),
        format!("{no_call}7")
    );
    assert_eq!(
        refused(
            "end-before",
            "    char *end = 0;\n    if (end) puts(\"set\");\n    long n = strtol(argv[1], &end, 10);\n    printf(\"%ld %d\\n\", n, *end);\n"
        ),
        format!("{no_call}7")
    );
    assert_eq!(
        refused(
            "end-loop",
            "    char *end = argv[1];\n    long n = 0;\n    while (*end) n += strtol(end, &end, 10);\n    printf(\"%ld\\n\", n);\n"
        ),
        format!("{no_call}7")
    );
    // Two empty arguments are two places in C, and Rust keeps no memory
    // for either.
    assert_eq!(
        refused(
            "end-two-arguments",
            "    char *a, *b;\n    strtol(argv[1], &a, 10);\n    strtol(argv[2], &b, 10);\n    printf(\"%d\\n\", a == b);\n"
        ),
        "unsupported: a comparison of the addresses of two program arguments, at line 8"
    );
    // An argument that the program chooses as it runs has no index yet.
    assert_eq!(
        refused(
            "end-any-argument",
            "    char *end;\n    long n = strtol(argv[argc - 1], &end, 10);\n    printf(\"%ld %d\\n\", n, *end);\n"
        ),
        "unsupported: an end pointer that is not an offset of one text that stays in its place, at line 6"
    );
    assert_eq!(
        refused(
            "end-base",
            "    char *end;\n    long n = strtol(argv[1], &end, 16);\n    printf(\"%ld %d\\n\", n, *end);\n"
        ),
        "unsupported: this call of the library's `strtol`: no rule yet, at line 6"
    );
}

#[test]
fn text_in_bytes_ends_at_its_nul() {
    let rust = check(
        "bytes",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
void show(const char *line) {
    if (line != NULL) {
        printf("[%s] %lu\n", line, strlen(line));
    }
}
int count(const char *text, char wanted) {
    int found = 0;
    for (int i = 0; text[i] != '\0'; i++) {
        if (text[i] == wanted) found++;
    }
    return found;
}
int main(int argc, char *argv[]) {
    char word[8] = "ab";
    char empty[4] = "";
    word[2] = 'c';
    word[3] = (char)('a' + argc);
    show(word);
    show("a literal");
    show(empty);
    show(&word[1]);
    show(NULL);
    puts(word);
    printf("%d %d %d\n", atoi(argv[1]), count("banana", 'a'), count(word, 'b'));
    return (int)strlen(word);
}
"#,
        &[&["7"], &["x"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn show(line: Option<&[u8]>) {",
            "println!(\"[{}] {}\", text_of(line), until_nul(line).len() as u64);",
            "fn count(text: &[u8], wanted: u8) -> i32 {",
            "show(Some(&word[..]));",
            "show(Some(&b\"a literal\\0\"[..]));",
            "show(Some(&word[1..]));",
            "println!(\"{}\", text_of(&word));",
            "count(b\"banana\\0\", b'a'),",
            "fn until_nul(bytes: &[u8]) -> &[u8] {",
        ],
    );
}

#[test]
fn strtol_says_where_the_number_ends() {
    let rust = check(
        "strtol-end",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
void show(const char *label, const char *text) {
    printf("%s=%s (%lu)\n", label, text, strlen(text));
}
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    char *end;
    long first = strtol(argv[1], &end, 10);
    if (end == argv[1]) {
        printf("no number\n");
        return 1;
    }
    if (*end != '\0') {
        printf("then %c\n", *end);
    }
    show("rest", end);
    int second = (int)strtoll(argv[2], &end, 10);
    if (end != argv[2] && *end == '\0') {
        printf("all of it\n");
    }
    show("second", argv[2]);
    printf("%ld %d\n", first, second);
    return 0;
}
"#,
        &[
            &["12", "34"],
            &["  -7xyz", "x"],
            &["abc", "1"],
            &["", "1"],
            &["-", "1"],
            &["+5 ", " 99999999999999999999"],
            &["0x10", "-0"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn show(label: &str, text: &str) {",
            "let (first, end) = parse_int_prefix(&args[1]);",
            "if end == 0 {",
            "if end != args[1].len() {",
            "println!(\"then {}\", args[1].as_bytes().get(end).copied().unwrap_or(0) as char);",
            "show(\"rest\", &args[1][end..]);",
            "let (second, end) = parse_int_prefix(&args[2]);",
            "let second = second as i32;",
            "if end != 0 && end == args[2].len() {",
            "show(\"second\", &args[2]);",
        ],
    );
}

#[test]
fn memset_and_strncpy_fill_and_copy_slices() {
    let rust = check(
        "strncpy",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
struct point { int x; double y; };
int main(int argc, char *argv[]) {
    int count = atoi(argv[1]);
    char source[20];
    char target[20] = "";
    char small[4];
    int numbers[6] = {1, 2, 3, 4, 5, 6};
    struct point point = {3, 4.5};
    memset(source, 'x', 19);
    source[19] = '\0';
    memset(source + 10, argc + '0', 3);
    if (count < 20) {
        strncpy(target, source, count);
        target[count] = '\0';
    }
    strncpy(small, "ab", sizeof(small));
    printf("%s|%s|%d%d%d\n", target, small, small[2], small[3], point.x);
    strncpy(small, "abcdefgh", 3);
    small[3] = '\0';
    memset(numbers, 0, 3 * sizeof(int));
    memset(&point, 0, sizeof(point));
    printf("%s %d %d %d %.1f\n", small, numbers[2], numbers[3], point.x, point.y);
    memset(numbers, 0, sizeof(numbers));
    return numbers[5];
}
"#,
        &[&["0"], &["5"], &["12"], &["19"], &["40"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "source[..19].fill(b'x');",
            "source[10..][..3].fill((args.len() as i32 + 48) as u8);",
            "copy_text(&mut target[..count as usize], &source);",
            "copy_text(&mut small, b\"ab\\0\");",
            "copy_text(&mut small[..3], b\"abcdefgh\\0\");",
            "numbers[..3].fill(0);",
            "point = Point::default();",
            "numbers.fill(0);",
        ],
    );
}

#[test]
fn the_argument_after_the_last_is_null() {
    // `argv[argc]` is NULL.
    let rust = check(
        "argv-null",
        r#"
#include <stdio.h>
int main(int argc, char **argv) {
    const char *name = NULL;
    if (argc < 4) name = argv[argc == 3 ? 1 : argc];
    if (name == NULL) puts("none"); else puts(name);
    return 0;
}
"#,
        &[&[], &["one"], &["one", "two"], &["one", "two", "three"]],
        "",
    );
    assert_lines(&rust, &["name = args", ".map(String::as_str);"]);
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "argv-null-test",
            "#include <stdio.h>\nint main(int argc, char **argv) {\n    printf(\"%d\\n\", argv[argc] == 0);\n    return 0;\n}\n"
        ),
        "unsupported: a test of a pointer expression with no rule, at line 3"
    );
    assert_eq!(
        refused(
            "argv-null-local",
            "#include <stdio.h>\nint main(int argc, char **argv) {\n    char *p = argv[argc];\n    puts(p == 0 ? \"null\" : \"non-null\");\n    return 0;\n}\n"
        ),
        "unsupported: a test for NULL of a pointer to a program argument: no rule yet, at line 4"
    );
}

#[test]
fn addresses_that_rust_does_not_tell_apart_are_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "address-null",
            "#include <stdio.h>\nint main(void) {\n    char *a = 0, *b = 0;\n    printf(\"%d\\n\", a == b);\n    return 0;\n}\n"
        ),
        "unsupported: a comparison of two addresses of which one may be NULL: no rule yet, at line 4"
    );
    // Two empty arguments are two places in C.
    assert_eq!(
        refused(
            "address-arguments",
            "#include <stdio.h>\nint main(int argc, char **argv) {\n    printf(\"%d\\n\", argv[1] == argv[2]);\n    return 0;\n}\n"
        ),
        "unsupported: a comparison of the addresses of two program arguments, at line 3"
    );
    assert_eq!(
        refused(
            "address-argument-variables",
            "#include <stdio.h>\nint main(int argc, char **argv) {\n    char *a = argv[1], *b = argv[2];\n    printf(\"%d\\n\", a == b);\n    return 0;\n}\n"
        ),
        "unsupported: a comparison of the addresses of two program arguments, at line 4"
    );
}

#[test]
fn the_end_of_a_number_is_an_index_into_what_was_read() {
    // Two numbers in one argument, the second read from the end of the
    // first; an end that nothing reads; and a variable that holds the
    // argument.
    let rust = check(
        "strtol-end-chain",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    char *end = NULL;
    long a = strtol(argv[1], &end, 10);
    long b = 0;
    b += strtol(end, &end, 10);
    printf("%ld %ld [%s] %d\n", a, b, end, (int)(end - argv[1]));
    const char *text = argv[2];
    int read = (int)strtol(text, &end, 10);
    printf("%d\n", read);
    long long c = strtoll(text, &end, 10);
    if (end == text || *end != '\0') {
        printf("not a number: %lld\n", c);
    }
    return 0;
}
"#,
        &[
            &["1+2x", "7"],
            &["7-8", "x"],
            &["5", ""],
            &["", "12 "],
            &["3+4+5", "-9223372036854775809"],
            &["\u{e9}1", "1\u{e9}"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let (a, end) = parse_int_prefix(&args[1]);",
            "let (parsed, read_) = parse_int_prefix(&args[1][end..]);",
            "let end: usize = end + read_;",
            "b += parsed;",
            "println!(\"{} {} [{}] {}\", a, b, &args[1][end..], end as i64 as i32);",
            "let text: &str = &args[2];",
            "let read = parse_int(text) as i32;",
            "let (c, end) = parse_int_prefix(text);",
            "if end == 0 || end != text.len() {",
        ],
    );
    // In bytes that the program reads, the end is an index like that of
    // any pointer that walks them.
    let rust = check(
        "strtol-end-bytes",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int number(const char *s, int *out) {
    char *end;
    long v = strtol(s, &end, 10);
    if (end == s || *end != '\0') return 0;
    *out = (int)v;
    return 1;
}
int main(void) {
    char buf[64];
    if (!fgets(buf, sizeof buf, stdin)) return 1;
    char *p = buf;
    long sum = 0;
    while (*p) {
        char *end;
        long v = strtol(p, &end, 10);
        if (end == p) break;
        sum += v;
        p = end;
        if (*p == ',') p++;
    }
    int out = -1;
    int found = number("23", &out);
    printf("%ld %d %d %d\n", sum, *p, found, out);
    return 0;
}
"#,
        &[&[]],
        "1,2,-3,x\n",
    );
    assert_lines(
        &rust,
        &[
            "fn number(s: &str, out: &mut i32) -> i32 {",
            "let (v, end) = parse_int_prefix(s);",
            "if end == 0 || end != s.len() {",
            "let (v, read) = parse_int_prefix(&text_of(&buf[p..]));",
            "let end: usize = p + read;",
            "if end == p {",
            "p = end;",
        ],
    );
}

#[test]
fn strtoul_and_strtod_say_where_their_numbers_end() {
    // A first value of the end that nothing reads is dropped.
    let rust = check(
        "strtoul-strtod-end",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    char *end = argv[1];
    unsigned long n = strtoul(argv[1], &end, 10);
    printf("%lu [%s]\n", n, end);
    end = NULL;
    double d = strtod(argv[2], &end);
    float f = strtof(end, &end);
    if (end == argv[2]) puts("no number");
    printf("%g %.9g [%s]\n", d, f, end);
    return 0;
}
"#,
        &[
            &["12", "1.5 2.5"],
            &["", "x"],
            &["-1x", " -2e3inf"],
            &["18446744073709551616", "1e400 1e-50"],
            &["  7", "nan(12)z"],
            &["+3", "1e 2."],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let (n, end) = parse_uint_prefix(&args[1]);",
            "let (d, end) = parse_float_prefix::<f64>(&args[2]);",
            "let (f, read) = parse_float_prefix::<f32>(&args[2][end..]);",
            "let end: usize = end + read;",
        ],
    );
    let refused = |name, c| translate(name, c).unwrap_err();
    // The end read before the call, or after a store of its own.
    for (name, line, c) in [
        (
            "end-read-first",
            6,
            "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char **argv) {\n    char *end = argv[1];\n    puts(end);\n    double d = strtod(argv[1], &end);\n    printf(\"%g %s\\n\", d, end);\n    return 0;\n}\n",
        ),
        (
            "end-read-after-store",
            5,
            "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char **argv) {\n    char *end;\n    double d = strtod(argv[1], &end);\n    end = argv[1];\n    printf(\"%g %s\\n\", d, end);\n    return 0;\n}\n",
        ),
    ] {
        assert_eq!(
            refused(name, c),
            format!(
                "unsupported: an end pointer other than a local `char *` that one call sets before each read: no rule yet, at line {line}"
            )
        );
    }
    assert_eq!(
        refused(
            "strtoul-base",
            "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char **argv) {\n    char *end;\n    unsigned long n = strtoul(argv[1], &end, 16);\n    printf(\"%lu %s\\n\", n, end);\n    return 0;\n}\n"
        ),
        "unsupported: this call of the library's `strtoul`: no rule yet, at line 5"
    );
}

#[test]
fn errno_after_a_number_is_whether_it_was_beyond_its_type() {
    // The edges of each type, and `inf`, `nan`, overflow and underflow of
    // a `double`, where glibc sets `ERANGE` and where it does not.
    let rust = check(
        "errno-range",
        r#"
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    if (argc != 4) return 2;
    char *end;
    errno = 0;
    long n = strtol(argv[1], &end, 10);
    if (errno == ERANGE) printf("long range %ld\n", n);
    else if (end == argv[1]) puts("no long");
    errno = 0;
    char *rest;
    unsigned long u = strtoul(argv[2], &rest, 10);
    int bad = errno != 0;
    printf("%lu %d %d %d\n", u, bad, !errno, errno == EDOM);
    errno = 0;
    double d = strtod(argv[3], &end);
    printf("%g %d [%s]\n", d, errno == ERANGE, end);
    errno = 0;
    float f = strtof(argv[3], &end);
    if (errno) printf("float range %g\n", f);
    return errno == ERANGE ? 3 : 0;
}
"#,
        &[
            &["12", "12", "1.5"],
            &["9223372036854775807", "18446744073709551615", "1e308"],
            &["9223372036854775808", "18446744073709551616", "1e309"],
            &["-9223372036854775808", "-18446744073709551615", "-1e309"],
            &["-9223372036854775809", "-18446744073709551616", "1e-310"],
            &["x", "-1", "1e-400"],
            &["", "", "inf"],
            &["-", "+", "nan"],
            &["7z", " 3", "1e-5"],
            &["0", "0", "0e-999"],
            &["1", "1", "1e-40"],
            &["1", "1", "3.5e38"],
            &["1", "1", "x"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let (n, end, range) = parse_int_checked(&args[1]);",
            "if range {",
            "let (u, _, range) = parse_uint_checked(&args[2]);",
            "let bad = range as i32;",
            "let (d, end, range) = parse_float_checked::<f64>(&args[3], f64::MIN_POSITIVE);",
            "let (f, _, range) = parse_float_checked::<",
        ],
    );
    let refused = |name, c: &str| translate(name, c).unwrap_err();
    let program = |lines: &str| {
        format!(
            "#include <errno.h>\n#include <stdio.h>\n#include <stdlib.h>\nint twice(int x) {{ return 2 * x; }}\nint main(int argc, char **argv) {{\n    char *end;\n{lines}\n    return 0;\n}}\n"
        )
    };
    let read = "unsupported: a read of `errno` that is not one call's error, at line";
    for (name, lines, message) in [
        (
            "errno-no-store",
            "    long n = strtol(argv[1], &end, 10);\n    if (errno == ERANGE) printf(\"%ld\\n\", n);",
            format!("{read} 8"),
        ),
        (
            "errno-call-between",
            "    errno = 0;\n    long n = strtol(argv[1], &end, 10);\n    int t = twice(1);\n    if (errno == ERANGE) printf(\"%ld %d\\n\", n, t);",
            format!("{read} 10"),
        ),
        (
            "errno-after-loop",
            "    long n = 0;\n    errno = 0;\n    for (int i = 1; i < argc; i++) n += strtol(argv[i], &end, 10);\n    if (errno) printf(\"%ld\\n\", n);",
            format!("{read} 10"),
        ),
        (
            "errno-atoi",
            "    errno = 0;\n    int n = atoi(argv[1]);\n    if (errno) printf(\"%d\\n\", n);",
            format!("{read} 9"),
        ),
        (
            "errno-value",
            "    errno = 0;\n    long n = strtol(argv[1], &end, 10);\n    printf(\"%ld %d\\n\", n, errno);",
            "unsupported: the number in `errno`, which is the platform's, at line 9".to_owned(),
        ),
        (
            "errno-store",
            "    errno = EINVAL;\n    long n = strtol(argv[1], &end, 10);\n    if (errno) printf(\"%ld\\n\", n);",
            "unsupported: a store to `errno` other than 0 before a call, at line 7".to_owned(),
        ),
    ] {
        assert_eq!(refused(name, &program(lines)), message, "{name}");
    }
}
