use crate::common::translate;

/// What has no rule is refused with its reason and its line. There is no
/// other translation to fall back on.
#[test]
fn a_construct_without_a_rule_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "union",
            "union both { int i; double d; };\nint main(void) { return 0; }\n"
        ),
        "unsupported: a union whose members are not all numbers of one size: no rule yet"
    );
    assert_eq!(
        refused(
            "goto",
            "int main(int argc, char *argv[]) {\n    if (argc-- > 1) goto end;\n    {\nend:\n        argc++;\n    }\n    return 0;\n}\n"
        ),
        "unsupported: `goto end` into a block from outside it: no rule yet"
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
    // `println!` has no value.
    assert_eq!(
        refused(
            "printed",
            "#include <stdio.h>\nint main(void) {\n    int n = printf(\"abc\\n\");\n    if (putchar('x') == 'x') n++;\n    return n;\n}\n"
        ),
        "unsupported: the value of `printf`: no rule yet, at line 3"
    );
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
    // The array ends where the function returns, and Rust lends nothing
    // that has ended.
    assert_eq!(
        refused(
            "returned",
            "#include <stdio.h>\nchar *name(void) {\n    char text[] = \"name\";\n    char *start = text;\n    return start;\n}\nint main(void) {\n    puts(name());\n    return 0;\n}\n"
        ),
        "unsupported: the result of `name`: it points to a variable that ends with the function"
    );
    // Rust lends memory once to a function that stores through it.
    assert_eq!(
        refused(
            "lent-twice",
            "void add(int *out, const int *in, int n) {\n    for (int i = 0; i < n; i++) out[i] += in[i];\n}\nint main(void) {\n    int a[4] = {1, 2, 3, 4};\n    add(a, a + 1, 3);\n    return a[0];\n}\n"
        ),
        "unsupported: two arguments that point into the same memory, one of them to store through, at line 6"
    );
}

/// A local pointer to a variable of the function is a borrow of it, so the
/// variable may not be stored to while the pointer is used later, nor read
/// while a pointer that is stored through is.
#[test]
fn a_variable_used_while_a_pointer_to_it_is_in_use_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "borrowed_record",
            "struct point { int row; int col; };\nint main(void) {\n    struct point at;\n    struct point *here;\n    here = &at;\n    at.row = 4;\n    here->col = 5;\n    return here->row + here->col;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 7"
    );
    assert_eq!(
        refused(
            "borrowed_rows",
            "int main(void) {\n    int cells[3][2], (*rows)[2] = cells;\n    int *corner = &cells[2][1];\n    cells[2][1] = 6;\n    return rows[2][1] + *corner - 12;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 5"
    );
    assert_eq!(
        refused(
            "borrowed_loop",
            "int main(void) {\n    int total = 0, step = 1;\n    int *into = &total;\n    for (int i = 0; i < 3; i++) {\n        *into += i;\n        total += step;\n    }\n    return total;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 5"
    );
    assert_eq!(
        refused(
            "borrowed_read",
            "int main(void) {\n    int level = 2;\n    int *raise = &level;\n    *raise = 3;\n    int seen = level;\n    *raise = seen + 1;\n    return level;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 6"
    );
}
