use crate::common::{assert_lines, check, translate};

#[test]
fn a_call_that_hands_one_pointer_twice_calls_a_copy_with_one_parameter() {
    let rust = check(
        "same",
        r#"
#include <stdio.h>
#include <stdlib.h>
void fma(int *out, const int *a, const int *b, const int *c, int n) {
    for (int i = 0; i < n; i++) out[i] = a[i] * b[i] + c[i];
}
void step(int *a, const int *a2, int *b, const int *b2) {
    a[0] += a2[0];
    b[0] *= b2[0];
}
void acc(int *out, const int *in, int n) {
    static int calls = 0;
    calls++;
    if (n == 0) return;
    out[n - 1] += in[n - 1] + calls;
    acc(out, in, n - 1);
}
void bump(int *to, const int *by) {
    if (by == NULL) return;
    *to += *by;
}
int main(int argc, char *argv[]) {
    int v[3];
    int w[1] = {argc};
    int x = 5;
    for (int i = 0; i < 3; i++) v[i] = argc + i;
    fma(v, v, v, v, 3);
    fma(v, v, w, v, 1);
    step(v, v, w, w);
    acc(v, v, 3);
    acc(v, w, 1);
    bump(&x, &x);
    bump(&x, NULL);
    printf("%d %d %d %d %d\n", v[0], v[1], v[2], w[0], x);
    return 0;
}
"#,
        &[&[], &["a"], &["a", "b"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fma_same(&mut v, 3);",
            "fma_same2(&mut v, &w, 1);",
            "step_same(&mut v, &mut w);",
            "acc_same(globals, &mut v, 3);",
            "acc(globals, &mut v, &w, 1);",
            "bump_same(&mut x);",
            "fn fma_same(out: &mut [i32], n: i32) {",
            "out[i as usize] = out[i as usize] * out[i as usize] + out[i as usize];",
            "acc_same(globals, out, n - 1);",
        ],
    );
}

#[test]
fn the_calls_that_hand_one_pointer_twice_with_no_copy_are_refused() {
    let lent =
        "unsupported: two arguments that point into the same memory, one of them to store through";
    // Two parameters that the function moves apart are not one variable.
    assert_eq!(
        translate(
            "same-moved",
            "void cp(char *d, const char *s) {\n    while (*s) *d++ = *s++;\n}\nint main(void) {\n    char t[4] = \"ab\";\n    cp(t, t);\n    return t[0];\n}\n"
        )
        .unwrap_err(),
        format!("{lent}, at line 6")
    );
    // A call with a side effect may not hand the same pointer twice.
    assert_eq!(
        translate(
            "same-impure",
            "int k;\nint next(void) { return ++k; }\nvoid sq(int *out, const int *in, int m) { out[0] = in[0] * m; }\nint main(void) {\n    int a[1] = {3};\n    sq(a, a, next());\n    return a[0];\n}\n"
        )
        .unwrap_err(),
        format!("{lent}, at line 6")
    );
    // Parameters of different types are not one parameter.
    assert_eq!(
        translate(
            "same-types",
            "void low(int *out, const unsigned char *in) { out[0] = in[0]; }\nint main(void) {\n    int a[1] = {258};\n    low(a, (unsigned char *)a);\n    return a[0];\n}\n"
        )
        .unwrap_err(),
        format!("{lent}, at line 4")
    );
    // The same field of one record is another family.
    assert_eq!(
        translate(
            "same-field",
            "struct buf { int data[2]; };\nvoid sq(int *out, const int *in) { out[0] = in[0] * 2; }\nint main(void) {\n    struct buf b = {{1, 2}};\n    struct buf *s = &b;\n    sq(s->data, s->data);\n    return b.data[0];\n}\n"
        )
        .unwrap_err(),
        format!("{lent}, at line 6")
    );
}
