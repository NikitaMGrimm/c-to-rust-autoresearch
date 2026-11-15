use crate::common::{assert_lines, check, translate};

const NUMBERS: &str = r#"
#include <stdio.h>
int main(void) {
    int data[8];
    int count = 0;
    while (count < 8) {
        if (scanf("%d", &data[count]) != 1) {
            break;
        }
        count++;
    }
    unsigned total = 0;
    short small = 7;
    long wide = -1;
    char letter = '?', next = '!';
    scanf(" %c%c", &letter, &next);
    if (scanf("%u %hd", &total, &small) == 2) {
        printf("both\n");
    } else {
        printf("not both\n");
    }
    if (fscanf(stdin, "%ld", &wide) < 1)
        printf("no long\n");
    else
        printf("a long\n");
    for (int i = 0; i < count; i++) {
        printf("%d ", data[i]);
    }
    printf("\n%c%c %u %d %ld\n", letter, next, total, small, wide);
    return count;
}
"#;

#[test]
fn scanf_reads_what_c_reads() {
    let mut rust = String::new();
    for stdin in [
        "1 2 3 x y 4000000000 -5 123456789012",
        "",
        "  -7\n+8 9\n\n",
        "1 2 3 4 5 6 7 8 9 10 11 12",
        "12abc 5",
        "- 3",
        "3 q\n\n z 5 x",
        "3 ab 5 6x",
        "99999999999999999999999 zz 1 2 -99999999999999999999999",
    ] {
        rust = check("scanf", NUMBERS, &[&[]], stdin);
    }
    assert_lines(
        &rust,
        &[
            "let Some(scanned) = scan_int() else {",
            "data[count as usize] = scanned as i32;",
            "if let Some(scanned) = read_byte() {",
            "letter = scanned;",
            "small = scanned as i16;",
            "if let Some(scanned) = scan_int() {",
            "wide = scanned;",
            "use std::io::BufRead;",
        ],
    );
}

#[test]
fn scanf_with_no_rule_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "scanf-size",
            "#include <stdio.h>\nint main(void) {\n    long x;\n    scanf(\"%d\", &x);\n    return 0;\n}\n"
        ),
        "unsupported: a `scanf` conversion for a variable of another size, at line 4"
    );
    assert_eq!(
        refused(
            "scanf-text",
            "#include <stdio.h>\nint main(void) {\n    int x;\n    scanf(\"x=%d\", &x);\n    return 0;\n}\n"
        ),
        "unsupported: text in a `scanf` format: no rule yet, at line 4"
    );
}

#[test]
fn fgets_reads_a_line_into_bytes() {
    let c = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
    char line[8] = "";
    char rest[32];
    int lines = 0;
    while (fgets(line, 8, stdin) != NULL) {
        lines++;
        printf("%d: %lu <%s>\n", atoi(line), strlen(line), line);
        if (lines == 3) break;
    }
    if (!fgets(rest, 4, stdin)) {
        puts("nothing left");
        return lines;
    }
    fgets(rest + 3, sizeof(rest) - 3, stdin);
    printf("rest <%s>\n", rest);
    return lines;
}
"#;
    let mut rust = String::new();
    for stdin in [
        "",
        "12\n",
        "12",
        "1234567890 and more\n-5\n\nlast line\nunread",
        "\n\n\nabc",
        "1\n2\n3\nabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
    ] {
        rust = check("fgets", c, &[&[]], stdin);
    }
    assert_lines(
        &rust,
        &[
            "while read_line(&mut line) {",
            "if !read_line(&mut rest[..4]) {",
            "read_line(&mut rest[3..][..29]);",
        ],
    );
}

#[test]
fn scanf_with_a_pointer_it_may_change_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    // C finds every pointer before it reads anything.
    assert_eq!(
        refused(
            "scanf-effect",
            "#include <stdio.h>\nint main(void) {\n    int a[1], i = 0;\n    scanf(\"%d\", &a[i++]);\n    printf(\"%d\\n\", i);\n    return 0;\n}\n"
        ),
        "unsupported: a `scanf` pointer that the call itself may change: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "scanf-order",
            "#include <stdio.h>\nint main(void) {\n    int a[4] = {0}, i = 0;\n    scanf(\"%d %d\", &i, &a[i]);\n    printf(\"%d %d\\n\", i, a[0]);\n    return 0;\n}\n"
        ),
        "unsupported: a `scanf` pointer that the call itself may change: no rule yet, at line 4"
    );
    // The first pointer is found before any store in the translation too.
    let rust = check(
        "scanf-first",
        "#include <stdio.h>\nint main(void) {\n    int a[4] = {0}, i = 1, j = 0;\n    scanf(\"%d %d\", &a[i], &j);\n    printf(\"%d %d\\n\", a[1], j);\n    return 0;\n}\n",
        &[&[]],
        "5 6",
    );
    assert_lines(&rust, &["a[i as usize] = scanned as i32;"]);
}

#[test]
fn fgets_with_a_size_beyond_its_buffer_reads_what_fits() {
    // C is right as long as the line is short, where a slice of the size
    // would stop the program at once.
    let c = r#"
#include <stdio.h>
void read(char *line, int size) {
    if (fgets(line, size, stdin) == NULL) {
        puts("nothing");
        return;
    }
    printf("<%s>\n", line);
}
int main(int argc, char **argv) {
    char a[2];
    char b[8];
    read(b, 8);
    read(b, 100);
    if (fgets(a, argc + 2, stdin)) printf("<%s>\n", a);
    if (fgets(b, 9, stdin)) printf("<%s>\n", b);
    return 0;
}
"#;
    let mut rust = String::new();
    for stdin in ["abc\nde\nx", "abc\n", "", "\n\n", "abc\nde\n"] {
        rust = check("fgets-size", c, &[&[]], stdin);
    }
    assert_lines(
        &rust,
        &[
            "if !read_line_within(line, size) {",
            "if read_line_within(&mut a, args.len() as i32 + 2) {",
            "if read_line_within(&mut b, 9) {",
        ],
    );
}

const COUNTS: &str = r#"
#include <stdio.h>
int last = 0;
int count(void) {
    return scanf("%d", &last) + 1;
}
int main(void) {
    int a = 0, b = 0, c = 0;
    char k = '?';
    int n = scanf("%d", &a);
    printf("one %d %d\n", n, a);
    n = scanf("%d %d", &a, &b);
    printf("two %d %d %d\n", n, a, b);
    n = scanf("%d%d%d", &a, &b, &c);
    printf("three %d %d %d %d\n", n, a, b, c);
    if (scanf("%d", &a) == EOF) {
        puts("end");
        return 3;
    }
    if (scanf("%d %d", &a, &b) > 0) puts("some");
    if (scanf("%d %d", &a, &b) >= 1) puts("at least one");
    if (scanf("%d%c", &a, &k) < 1) puts("none");
    if (scanf(" %d", &a) == 0) puts("no number");
    if (scanf("%d", &a) < 5) puts("always");
    if (scanf("%d %d", &a, &b) == 1) puts("just one");
    printf("%d\n", count());
    printf("%d\n", last);
    switch (scanf("%d %d %d", &a, &b, &c)) {
    case -1:
        puts("switch end");
        break;
    case 0:
        puts("switch none");
        break;
    case 3:
        puts("switch all");
        break;
    default:
        puts("switch some");
        break;
    }
    printf("%d %d %d %c\n", a, b, c, k);
    unsigned u = scanf("%c", &k);
    printf("%u %d\n", u, scanf(" %c", &k));
    printf("%c\n", k);
    while (scanf("%d", &a) == 1) printf("%d,", a);
    while (scanf("%c", &k) != EOF) printf("[%c]", k);
    printf("\n");
    return scanf("%d", &last);
}
"#;

#[test]
fn scanf_gives_its_count_as_a_value() {
    let mut rust = String::new();
    for stdin in [
        "",
        "   \n\t",
        "5",
        "5\n",
        "abc",
        "-",
        "- 3",
        "+x",
        "12abc",
        "1 2",
        "1 2 3 4 5 6",
        "1 x 2 y 3 z",
        "1\n2\n3\n4 5 6 7 8 9 10 11 12 13 14 15 16 17 abc",
        "1 2 3 4 5 6 7 8 9 10 - 11 12 13 14 15 16 17 18 +",
        "1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 ab",
        "1 2 3 4 5 6 7 8 9 10 11 x",
    ] {
        rust = check("scanf-count", COUNTS, &[&[]], stdin);
    }
    assert_lines(
        &rust,
        &[
            "let mut n: i32 = match scan_int_or_end() {",
            "Err(ScanError::End) => -1_i32,",
            "Err(ScanError::NoMatch) => 0,",
            "} else {",
            "if let Some(scanned) = scan_int() {",
            "while if let Some(scanned) = scan_int() {",
            "-1 => {",
            "} < 5",
            "} as u32;",
            "fn scan_int_or_end() -> Result<i64, ScanError> {",
        ],
    );
}

#[test]
fn scanf_of_a_character_gives_its_count() {
    let c = r#"
#include <stdio.h>
struct Result {
    char last;
};
struct Result result = {'?'};
int next(void) {
    return scanf(" %c", &result.last) + 1;
}
int main(void) {
    char a = '?', b = '?';
    int x = 0;
    int n = scanf("%c", &a);
    printf("%d %c\n", n, a);
    n = scanf("%d%c", &x, &b);
    printf("%d %d %c\n", n, x, b);
    n = scanf(" %c %c", &a, &b);
    printf("%d %c %c\n", n, a, b);
    while (scanf("%c", &a) > 0 && a != '.') printf("[%c]", a);
    printf("%d %c\n", next(), result.last);
    return n == EOF;
}
"#;
    for stdin in [
        "", "a", "ab", "a5", "a5x", "a5\n", "a 5 x  y", "  ", "a+ x", "a-\n\n",
    ] {
        check("scanf-char-count", c, &[&[]], stdin);
    }
}

#[test]
fn scanf_and_other_readers_are_refused_as_values() {
    let refused = |c: &str| translate("scanf-value-refused", c).unwrap_err();
    let program = |call: &str| {
        format!(
            "#include <stdio.h>\nint main(void) {{\n    int x; char s[8]; float f;\n    int n = {call};\n    return n;\n}}\n"
        )
    };
    for (call, message) in [
        ("scanf(\"%*d\")", "the `scanf` conversion `%*`: no rule yet"),
        (
            "scanf(\"%s\", s)",
            "the `scanf` conversion `%s`: no rule yet",
        ),
        (
            "scanf(\"%[a-z]\", s)",
            "the `scanf` conversion `%[`: no rule yet",
        ),
        (
            "scanf(\"%d%n\", &x, &x)",
            "the `scanf` conversion `%n`: no rule yet",
        ),
        (
            "scanf(\"%f\", &f)",
            "the `scanf` conversion `%f`: no rule yet",
        ),
        (
            "scanf(\"%3d\", &x)",
            "the `scanf` conversion `%3`: no rule yet",
        ),
        (
            "sscanf(\"1\", \"%d\", &x)",
            "this call of the library's `sscanf`: no rule yet",
        ),
        (
            "fscanf(stderr, \"%d\", &x)",
            "this call of the library's `fscanf`: no rule yet",
        ),
    ] {
        assert_eq!(
            refused(&program(call)),
            format!("unsupported: {message}, at line 4"),
            "{call}"
        );
    }
}
