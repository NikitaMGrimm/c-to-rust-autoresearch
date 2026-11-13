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
    assert_eq!(
        refused(
            "end-null",
            "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char **argv) {\n    char *end = 0;\n    long n = strtol(argv[1], &end, 10);\n    printf(\"%ld %d\\n\", n, *end);\n    return 0;\n}\n"
        ),
        "unsupported: an end pointer that may be NULL: no rule yet, at line 5"
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
            "let mut end: &str = \"\";",
            "let first: i64 = parse_int_rest(&args[1], &mut end);",
            "if end.as_ptr() == args[1].as_ptr() {",
            "if !end.is_empty() {",
            "println!(\"then {}\", end.bytes().next().unwrap_or(0) as char);",
            "show(\"rest\", end);",
            "if end.as_ptr() != args[2].as_ptr() && end.is_empty() {",
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
