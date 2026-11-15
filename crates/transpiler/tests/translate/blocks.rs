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
        &["if !(args.len() as i32 > 0) {", "std::process::abort();"],
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
            r#"program.c:6: int half(int): Assertion `n % 2 == 0 && \"{{even}}\"' failed.","#
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
