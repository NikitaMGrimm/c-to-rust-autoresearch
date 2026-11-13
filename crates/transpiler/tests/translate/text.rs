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
fn text_that_is_read_byte_by_byte_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "text-bytes",
            "int main(void) {\n    const char *text = \"abc\";\n    return text[1];\n}\n"
        ),
        "unsupported: a string literal as a pointer to bytes: no rule yet, at line 2"
    );
    assert_eq!(
        refused(
            "text-width",
            "#include <stdio.h>\nint main(void) {\n    const char *text = \"abc\";\n    printf(\"%5s\", text);\n    return 0;\n}\n"
        ),
        "unsupported: a `printf` width for text in a variable, at line 4"
    );
}
