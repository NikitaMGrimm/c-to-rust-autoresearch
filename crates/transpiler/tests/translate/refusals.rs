use crate::common::translate;

/// What has no rule is refused with its reason and its line. There is no
/// other translation to fall back on.
#[test]
fn a_construct_without_a_rule_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "union",
            "union both { int i; float f; };\nint main(void) { return 0; }\n"
        ),
        "unsupported: the union `both`"
    );
    assert_eq!(
        refused(
            "goto",
            "int main(void) {\n    goto end;\nend:\n    return 0;\n}\n"
        ),
        "unsupported: `goto end`: no rule yet"
    );
    assert_eq!(
        refused(
            "addresses",
            "int main(void) {\n    int a, b;\n    int *p = &a, *q = &b;\n    return p < q;\n}\n"
        ),
        "unsupported: a comparison of two addresses, at line 4"
    );
}

/// What a rule would translate wrongly is refused too.
#[test]
fn a_program_that_a_rule_would_get_wrong_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    // Rust would step `i` twice.
    assert_eq!(
        refused(
            "twice",
            "int main(void) {\n    int a[3] = {1, 2, 3}, i = 0;\n    return a[i++]++;\n}\n"
        ),
        "unsupported: a place that calls or assigns and is updated, at line 3"
    );
    assert_eq!(
        refused(
            "tags",
            "struct s { int a; };\nint main(void) {\n    struct s { int b; };\n    struct s x = {7};\n    return x.b;\n}\n"
        ),
        "unsupported: the tag `s` with two meanings, at line 3"
    );
    assert_eq!(
        refused(
            "typedefs",
            "typedef int T;\nT g = 0;\nint main(void) {\n    typedef unsigned char T;\n    T n = 1;\n    return n + g;\n}\n"
        ),
        "unsupported: the typedef `T` with two meanings, at line 4"
    );
    assert_eq!(
        refused(
            "packed",
            "struct __attribute__((packed)) s { char a; int b; };\nint main(void) { return sizeof(struct s) == 5; }\n"
        ),
        "unsupported: a record that is packed or aligned"
    );
    assert_eq!(
        refused(
            "wide",
            "enum e { BIG = 1099511627776L };\nint main(void) { return sizeof(enum e) == 8; }\n"
        ),
        "unsupported: the type `enum e`, at line 2"
    );
    assert_eq!(
        refused(
            "width",
            "#include <stdio.h>\nint main(void) {\n    printf(\"|%4s|\\n\", \"\u{e9}\");\n    return 0;\n}\n"
        ),
        "unsupported: a `printf` width for text that is not ASCII, at line 3"
    );
}
