use crate::common::{assert_lines, check, translate};

#[test]
fn an_assert_that_holds_is_a_test() {
    let rust = check(
        "assert-holds",
        r#"
#include <assert.h>
#include <stdio.h>
int main(int argc, char **argv) {
    assert(argc > 0);
    printf("%d\n", argc);
    return 0;
}
"#,
        &[&[], &["a"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "assert_holds(",
            "args.len() as i32 > 0,",
            "fn assert_holds(holds: bool, message: &str) {",
            "std::process::abort();",
        ],
    );
}

#[test]
fn an_assert_that_fails_aborts() {
    let rust = check(
        "assert-fails",
        r#"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
static int half(int n) {
    assert(n % 2 == 0 && "{even}");
    return n / 2;
}
int main(int argc, char **argv) {
    int n = half(argc > 1 ? atoi(argv[1]) : 4);
    printf("%d\n", n);
    return 0;
}
"#,
        &[&[], &["6"], &["7"]],
        "",
    );
    assert!(
        rust.contains(
            r#"program.c:6: int half(int): Assertion `n % 2 == 0 && \"{even}\"' failed.","#
        ),
        "{rust}"
    );
}

#[test]
fn the_condition_of_an_assert_runs_once() {
    let rust = check(
        "assert-once",
        r#"
#include <assert.h>
#include <stdio.h>
static int calls;
static int next(void) { calls++; return calls; }
int main(void) {
    int n = 0;
    assert(next() > 0);
    assert(++n == 1);
    printf("%d %d\n", calls, n);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(&rust, &["std::process::abort();"]);
}

#[test]
fn an_assert_on_a_pointer_tests_it() {
    let rust = check(
        "assert-pointer",
        r#"
#include <assert.h>
#include <stdio.h>
static int first(const int *p) {
    assert(p);
    return *p;
}
int main(int argc, char **argv) {
    int a[2] = {argc, 2};
    int *p = argc > 2 ? NULL : a;
    int n = first(a);
    assert(p != NULL && a[1] == 2);
    printf("%d %d\n", n, *p);
    return 0;
}
"#,
        &[&[], &["a", "b"]],
        "",
    );
    assert_lines(&rust, &["std::process::abort();"]);
}

#[test]
fn an_assert_before_a_comma_is_a_statement() {
    let rust = check(
        "assert-comma",
        r#"
#include <assert.h>
#include <stdio.h>
int main(int argc, char **argv) {
    int n = (assert(argc < 3), argc + 1);
    printf("%d\n", n);
    return 0;
}
"#,
        &[&[], &["a", "b"]],
        "",
    );
    assert_lines(&rust, &["std::process::abort();"]);
}

#[test]
fn an_assert_without_checks_is_nothing() {
    let rust = check(
        "assert-ndebug",
        r#"
#define NDEBUG
#include <assert.h>
#include <stdio.h>
int main(int argc, char **argv) {
    assert(argc > 5);
    printf("%d\n", argc);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert!(
        !rust.contains("abort") && !rust.contains("argc > 5"),
        "{rust}"
    );
}

#[test]
fn an_assert_has_no_value() {
    let refused = translate(
        "assert-value",
        r#"
#include <assert.h>
int main(int argc, char **argv) {
    argc > 1 ? assert(argc < 4) : (void)0;
    return 0;
}
"#,
    );
    assert_eq!(
        refused.unwrap_err(),
        "unsupported: an `assert` where a value is needed, at line 4"
    );
}

#[test]
fn a_statement_expression_is_a_block() {
    let rust = check(
        "block-value",
        r#"
#include <stdio.h>
static int twice(int a) { return a * 2; }
static int square(int a) { return ({ int b = a + 3; b * b; }); }
int main(int argc, char **argv) {
    int x = ({ int y = twice(argc); y + 1; });
    int z = twice(({ int t = argc + 1; t; })) + ({ int u = argc; u * 10; });
    int n = ({ int a = ({ int b = argc; b + 1; }); a * 2; });
    if (({ int t = argc % 2; t == 0; }))
        printf("even\n");
    int k = argc + 3, steps = 0;
    while (({ k--; k > 0; }))
        steps++;
    printf("%d %d %d %d %d\n", x, z, n, square(argc), steps);
    return 0;
}
"#,
        &[&[], &["a"], &["a", "b"]],
        "",
    );
    assert_lines(
        &rust,
        &["let y: i32 = twice(args.len() as i32);", "y + 1", "b * b"],
    );
}

#[test]
fn a_statement_expression_is_a_scope_and_stores_to_outer_variables() {
    let rust = check(
        "block-scope",
        r#"
#include <stdio.h>
int main(int argc, char **argv) {
    int x = 1;
    int y = ({ int x = 5 + argc; x + 1; }) + x;
    int w = ({ x = argc + 2; x * 2; });
    int r = ({ int found = -1; for (int i = 0; i < 10; i++) { if (i * argc > 4) { found = i; break; } } found; });
    int a = argc;
    ({ a += argc; a + 1; });
    printf("%d %d %d %d %d\n", x, y, w, r, a);
    return 0;
}
"#,
        &[&[], &["a"], &["a", "b", "c"]],
        "",
    );
    assert_lines(&rust, &["let x = 5 + args.len() as i32;", "break;"]);
}

#[test]
fn a_statement_expression_that_leaves_or_gives_a_pointer_is_refused() {
    let refused = |name, body: &str| {
        translate(
            name,
            &format!("int main(int argc, char **argv) {{\n{body}\n}}\n"),
        )
        .unwrap_err()
    };
    let cases = [
        (
            "block-return",
            "int r = ({ if (argc > 1) return 9; argc; }); return r;",
            "a `return`",
        ),
        (
            "block-break",
            "int s = 0; for (int i = 0; i < 5; i++) s += ({ if (i == argc) break; i; }); return s;",
            "a `break`",
        ),
        (
            "block-continue",
            "int s = 0; for (int i = 0; i < 5; i++) s += ({ if (i == argc) continue; i; }); return s;",
            "a `continue`",
        ),
        (
            "block-goto",
            "int s = ({ if (argc > 1) goto out; 3; }); return s;\nout: return 0;",
            "a `goto`",
        ),
    ];
    for (name, body, jump) in cases {
        assert_eq!(
            refused(name, body),
            format!("unsupported: a jump out of a statement expression, {jump}, at line 2")
        );
    }
    let cases = [
        (
            "block-pointer",
            "int a[2] = {1, 2}; int *p = ({ int *q = a; q + 1; }); return *p;",
            "a statement expression that gives a pointer",
        ),
        (
            "block-record",
            "struct pt { int x; } p = ({ struct pt q = {argc}; q; }); return p.x;",
            "a statement expression that gives a record",
        ),
        (
            "block-static",
            "return ({ static int c = 3; c + argc; });",
            "a `static` local in a statement expression",
        ),
        (
            "block-unset",
            "return ({ int *p; p = &argc; *p; });",
            "a pointer without a first value in a statement expression",
        ),
        (
            "block-void",
            "argc > 1 ? ({ argc = 0; (void)0; }) : (void)0; return argc;",
            "a statement expression without a value",
        ),
    ];
    for (name, body, reason) in cases {
        assert_eq!(
            refused(name, body),
            format!("unsupported: {reason}, at line 2")
        );
    }
}
