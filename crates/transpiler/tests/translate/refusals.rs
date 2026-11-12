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
