use crate::common::{assert_lines, check, translate};

const RUNS: &[&[&str]] = &[&["3"], &["7"]];

/// The program refused with the reason, at its line.
fn refused(name: &str, c: &str) -> String {
    translate(name, c).unwrap_err()
}

#[test]
fn a_void_parameter_that_holds_bytes_is_the_bytes() {
    let rust = check(
        "void-bytes",
        r#"
#include <stdio.h>
#include <stdlib.h>
static unsigned sum(void *p, size_t len) {
    unsigned char *d = (unsigned char *)p;
    unsigned s = 0;
    for (size_t i = 0; i < len; i++) s = s * 31 + d[i];
    return s;
}
static unsigned outer(void *p, size_t len) { return sum(p, len); }
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    unsigned char mem[16];
    for (int i = 0; i < 16; i++) mem[i] = (unsigned char)(i * atoi(argv[1]));
    printf("%u\n", outer(mem, 16));
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn sum(p: &[u8], len: u64) -> u32 {",
            "let d: &[u8] = p;",
            "fn outer(p: &[u8], len: u64) -> u32 {",
            "sum(p, len)",
            "println!(\"{}\", outer(&mem, 16));",
        ],
    );
}

#[test]
fn a_void_parameter_that_carries_one_record_is_a_borrow_of_it() {
    let rust = check(
        "void-context",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct acc { int total; int count; };
static void add(void *ctx, int value) {
    struct acc *a = (struct acc *)ctx;
    a->total += value;
    a->count++;
}
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    struct acc a = {0, 0};
    for (int i = 0; i < atoi(argv[1]); i++) add(&a, i * i);
    printf("%d %d\n", a.total, a.count);
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn add(ctx: &mut Acc, value: i32) {",
            "let a: &mut Acc = &mut *ctx;",
            "add(&mut a, i * i);",
        ],
    );
}

#[test]
fn a_void_result_field_and_global_take_the_type_they_hold() {
    let rust = check(
        "void-places",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct item { int key; void *data; };
static int a = 1, b = 2;
static void *store;
static void *choose(int n) { return n > 5 ? (void *)&a : (void *)&b; }
static void fill(int n) { double *d = store; *d = n / 4.0; }
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    int n = atoi(argv[1]);
    const int *p = choose(n);
    struct item it;
    it.key = 4;
    it.data = malloc(sizeof(int));
    *(int *)it.data = n;
    store = malloc(sizeof(double));
    fill(n);
    printf("%d %d %.2f\n", *p, it.key + *(int *)it.data, *(double *)store);
    free(store);
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "data: Box<i32>,",
            "store: Option<Box<f64>>,",
            "fn choose(n: i32) -> &'static i32 {",
            "if n > 5 { &A } else { &B }",
            "let mut d: Option<&mut f64> = globals.store.as_deref_mut();",
            "it.data = Box::new(0);",
            "*it.data = n;",
        ],
    );
}

#[test]
fn slots_that_hand_a_void_pointer_on_share_its_type() {
    let rust = check(
        "void-handed",
        r#"
#include <stdio.h>
#include <stdlib.h>
static void *first(void *a) { return a; }
static int pick(void *p) { return *(int *)p; }
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    long a = atol(argv[1]);
    void *p, *q;
    p = &a;
    q = p;
    long *r = q;
    int x = 10, y = 20;
    void *c = a > 5 ? (void *)&x : (void *)&y;
    int *f = first(&x);
    printf("%ld %d %d\n", *r * 3, pick(c), *f);
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "let p: &i64 = &a;",
            "let q: &i64 = p;",
            "let r: &i64 = q;",
            "let c: &i32 = if a > 5 { &x } else { &y };",
            "fn first(a: &i32) -> &i32 {",
            "fn pick(p: &i32) -> i32 {",
            "let f: &i32 = first(&x);",
        ],
    );
}

#[test]
fn a_conversion_of_a_void_pointer_is_the_pointer_itself() {
    let rust = check(
        "void-conversions",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct point { int x; int y; };
static int len2(const void *p) { return ((const struct point *)p)->x * ((struct point *)p)->y; }
static int twice(void *p) { return *(int *)p * 2; }
static void set(void *p, int v) { int *q = p; *q = v + 1; }
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    struct point pt = {atoi(argv[1]), 4};
    int x = 0;
    set(&x, pt.x);
    printf("%d %d\n", len2(&pt), twice(&x));
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn len2(p: &Point) -> i32 {",
            "p.x * p.y",
            "fn twice(p: &i32) -> i32 {",
            "*p * 2",
            "fn set(p: &mut i32, v: i32) {",
            "let q: &mut i32 = &mut *p;",
        ],
    );
}

#[test]
fn a_void_pointer_that_may_be_null_is_an_option() {
    let rust = check(
        "void-null",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int read_or(void *p, int fallback) {
    if (p == NULL) return fallback;
    return *(int *)p;
}
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    int x = atoi(argv[1]);
    void *p = x > 5 ? &x : NULL;
    printf("%d %d\n", read_or(p, -1), read_or(NULL, -1));
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn read_or(p: Option<&i32>, fallback: i32) -> i32 {",
            "let Some(p) = p else {",
            "let p: Option<&i32> = if x > 5 { Some(&x) } else { None };",
        ],
    );
}

#[test]
fn an_allocation_kept_in_a_void_pointer_is_of_the_type_it_is_converted_to() {
    let rust = check(
        "void-heap",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    int n = atoi(argv[1]);
    void *one = malloc(sizeof(int));
    int *p = (int *)one;
    *p = n;
    void *m = calloc(n, sizeof(int));
    int *v = m;
    for (int i = 0; i < n; i++) v[i] = i * i;
    printf("%d %d\n", *p * 2, v[n - 1]);
    free(p);
    free(m);
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "let one: Box<i32> = Box::new(0);",
            "let mut p: Box<i32> = one;",
            "let mut m: Vec<i32> = vec![0; n as usize];",
            "let v: &mut [i32] = &mut m;",
        ],
    );
}

#[test]
fn the_library_takes_the_type_of_its_void_arguments() {
    let rust = check(
        "void-library",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static void clear(void *p, size_t len) { memset(p, 0, len); }
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    unsigned char buf[8];
    for (int i = 0; i < 8; i++) buf[i] = (unsigned char)(i + atoi(argv[1]));
    clear(buf, 4);
    int a[4] = {1, 2, 3, 4}, b[4];
    memcpy(b, a, sizeof a);
    printf("%d %d %d %d\n", buf[0], buf[5], b[3], memcmp(a, b, sizeof a) == 0);
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &["fn clear(p: &mut [u8], len: u64) {", "clear(&mut buf, 4);"],
    );
}

#[test]
fn a_size_beside_a_void_pointer_stays_the_programs_number() {
    let rust = check(
        "void-size",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int sum(const void *p, size_t len) {
    const int *v = p;
    int s = 0;
    for (size_t i = 0; i < len; i++) s += v[i];
    return s;
}
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    int a[5] = {1, 2, 3, 4, 5};
    a[0] = atoi(argv[1]);
    printf("%d\n", sum(a, 5));
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &["fn sum(p: &[i32], len: u64) -> i32 {", "for i in 0..len {"],
    );
}

#[test]
fn a_void_pointer_whose_type_nothing_says_is_refused() {
    assert_eq!(
        refused(
            "void-untyped",
            "#include <stddef.h>\nstatic void *pass(void *p) { return p; }\nint main(void) {\n    void *p = NULL;\n    return pass(p) == NULL;\n}\n"
        ),
        "unsupported: a `void` pointer whose type nothing says, at line 2"
    );
}

#[test]
fn a_void_pointer_that_holds_two_types_is_refused() {
    assert_eq!(
        refused(
            "void-bytes-of-int",
            "static int first(const void *p) { const unsigned char *b = p; return b[0]; }\nint main(void) {\n    int x = 1;\n    return first(&x);\n}\n"
        ),
        "unsupported: a `void` pointer that holds an `unsigned char` and an `int`, at line 1"
    );
    assert_eq!(
        refused(
            "void-tagged",
            "struct a { int v; };\nstruct b { double w; };\nstatic double value(int tag, void *p) {\n    if (tag) return ((struct a *)p)->v;\n    return ((struct b *)p)->w;\n}\nint main(void) {\n    struct a x = {1};\n    return (int)value(1, &x);\n}\n"
        ),
        "unsupported: a `void` pointer that holds a `struct a` and a `struct b`, at line 3"
    );
}

#[test]
fn char_signed_char_and_unsigned_char_are_two_types_in_a_void_pointer() {
    assert_eq!(
        refused(
            "void-char",
            "static int first(void *p) { unsigned char *u = p; return u[0]; }\nint main(void) {\n    char text[4] = \"abc\";\n    return first(text);\n}\n"
        ),
        "unsupported: a `void` pointer that holds an `unsigned char` and a `char`, at line 1"
    );
    assert_eq!(
        refused(
            "void-signed-char",
            "static int first(void *p) { signed char *s = p; return s[0]; }\nint main(void) {\n    unsigned char bytes[2] = {200, 1};\n    return first(bytes);\n}\n"
        ),
        "unsupported: a `void` pointer that holds a `signed char` and an `unsigned char`, at line 1"
    );
}

#[test]
fn a_function_kept_in_a_void_pointer_is_refused() {
    assert_eq!(
        refused(
            "void-function",
            "#include <stddef.h>\nstatic int one(int x) { return x + 1; }\nint main(void) {\n    void *f = (void *)one;\n    return f != NULL;\n}\n"
        ),
        "unsupported: a function kept in a `void` pointer: no rule yet, at line 4"
    );
}

#[test]
fn arithmetic_on_a_void_pointer_and_its_size_are_refused() {
    assert_eq!(
        refused(
            "void-arithmetic",
            "int main(void) {\n    unsigned char mem[4] = {1, 2, 3, 4};\n    void *p = mem;\n    p = p + 1;\n    return *(unsigned char *)p;\n}\n"
        ),
        "unsupported: arithmetic on a `void` pointer: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "void-size-of",
            "int main(void) {\n    int x = 1;\n    void *p = &x;\n    return (int)sizeof(void) + *(int *)p;\n}\n"
        ),
        "unsupported: the size of `void`, at line 4"
    );
    assert_eq!(
        refused(
            "void-read",
            "int main(void) {\n    int x = 1;\n    void *p = &x;\n    (void)*p;\n    return *(int *)p;\n}\n"
        ),
        "unsupported: a read through a `void` pointer, at line 4"
    );
}

#[test]
fn a_declaration_before_the_definition_makes_no_slot_of_its_own() {
    let rust = check(
        "void-prototype",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int twice(void *p);
static void *get(void *);
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    int x = atoi(argv[1]);
    printf("%d\n", twice(get(&x)));
    return 0;
}
static void *get(void *q) { return q; }
static int twice(void *p) { return *(int *)p * 2; }
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &["fn get(q: &i32) -> &i32 {", "fn twice(p: &i32) -> i32 {"],
    );
}

#[test]
fn a_void_pointer_behind_a_pointer_or_in_an_array_is_refused() {
    assert_eq!(
        refused(
            "void-pointer-pointer",
            "static void put(void **out, void *value) { *out = value; }\nint main(void) {\n    int x = 1;\n    void *slot = 0;\n    put(&slot, &x);\n    return *(int *)slot;\n}\n"
        ),
        "unsupported: a pointer to a pointer, at line 1"
    );
    assert_eq!(
        refused(
            "void-array",
            "int main(void) {\n    int x = 1, y = 2;\n    void *items[2] = {&x, &y};\n    return *(int *)items[0];\n}\n"
        ),
        "unsupported: a pointer in an array, at line 3"
    );
}

#[test]
fn a_void_pointer_printed_with_percent_p_is_refused() {
    assert_eq!(
        refused(
            "void-percent-p",
            "#include <stdio.h>\nint main(void) {\n    int x = 1;\n    printf(\"%p\\n\", (void *)&x);\n    return 0;\n}\n"
        ),
        "unsupported: the `printf` conversion `%p`: no rule yet, at line 4"
    );
}
