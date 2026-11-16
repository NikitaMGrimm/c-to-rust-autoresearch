use crate::common::{assert_lines, check, translate};

const OPS: &str = r#"
#include <stdio.h>
#include <stdlib.h>
static int twice(int x) { return 2 * x; }
static int square(int x) { return x * x; }
static int negate(int x) { return -x; }
"#;

fn program(main: &str) -> String {
    format!("{OPS}{main}")
}

const RUNS: &[&[&str]] = &[&["0"], &["1"], &["2"], &["5"]];

#[test]
fn a_pointer_to_a_function_is_an_fn_value() {
    let rust = check(
        "fn_local",
        &program(
            r#"
typedef int (*unary)(int);
typedef int op_t(int);
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    int (*op)(int) = k % 2 ? twice : square;
    unary other = negate;
    op_t *third = twice;
    if (k > 1) other = op;
    printf("%d %d %d\n", op(k + 1), other(k), third(k));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "let op: fn(i32) -> i32 = if k % 2 != 0 { twice } else { square };",
            "let mut other: fn(i32) -> i32 = negate;",
            "let third: fn(i32) -> i32 = twice;",
            "other = op;",
            r#"println!("{} {} {}", op(k + 1), other(k), third(k));"#,
        ],
    );
}

#[test]
fn a_parameter_written_as_a_function_is_a_pointer() {
    let rust = check(
        "fn_param",
        &program(
            r#"
static int apply(int (*f)(int), int x) { return f(x) + 1; }
static int again(int f(int), int x) { return f(f(x)); }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    printf("%d %d\n", apply(twice, k), again(k > 1 ? negate : square, k));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn apply(f: fn(i32) -> i32, x: i32) -> i32 {",
            "fn again(f: fn(i32) -> i32, x: i32) -> i32 {",
            "f(f(x))",
            r#"println!("{} {}", apply(twice, k), again(if k > 1 { negate } else { square }, k));"#,
        ],
    );
}

#[test]
fn a_function_returns_a_pointer_to_a_function() {
    let rust = check(
        "fn_result",
        &program(
            r#"
typedef int (*unary)(int);
static int (*pick(int k))(int) { if (k == 0) return twice; if (k == 1) return square; return negate; }
static unary choose(int k) { return k % 2 ? twice : negate; }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    int (*f)(int) = pick(k);
    printf("%d %d\n", f(7), choose(k)(k + 2));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn pick(k: i32) -> fn(i32) -> i32 {",
            "fn choose(k: i32) -> fn(i32) -> i32 {",
            "let f: fn(i32) -> i32 = pick(k);",
            r#"println!("{} {}", f(7), choose(k)(k + 2));"#,
        ],
    );
}

#[test]
fn a_record_with_a_pointer_to_a_function_stays_copy() {
    let rust = check(
        "fn_field",
        &program(
            r#"
struct op { int bias; int (*run)(int); };
static int use(struct op o, int x) { return o.run(x) + o.bias; }
static void call(struct op *o, int x) { o->bias++; printf("%d\n", o->run(x)); }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    struct op a = { 1, twice };
    struct op b = a;
    b.run = negate;
    call(&b, k);
    printf("%d %d\n", use(a, k), use(b, k));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "#[derive(Clone, Copy)]",
            "run: fn(i32) -> i32,",
            "(o.run)(x) + o.bias",
            r#"println!("{}", (o.run)(x));"#,
            "let a = Op { bias: 1, run: twice };",
            "b.run = negate;",
        ],
    );
    assert!(!rust.contains("impl Default"), "{rust}");
}

#[test]
fn a_global_pointer_to_a_function_is_a_constant_or_a_field_of_globals() {
    let rust = check(
        "fn_global",
        &program(
            r#"
static int (*const chosen)(int) = square;
static int (*current)(int) = twice;
static void choose(int k) { if (k > 1) current = negate; }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    choose(k);
    printf("%d %d\n", chosen(k), current(k));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "const CHOSEN: fn(i32) -> i32 = square;",
            "current: fn(i32) -> i32,",
            "globals.current = negate;",
            r#"println!("{} {}", CHOSEN(k), (globals.current)(k));"#,
        ],
    );
}

#[test]
fn an_array_of_pointers_to_functions_is_indexed_and_called() {
    let rust = check(
        "fn_array",
        &program(
            r#"
typedef int (*unary)(int);
static const unary table[] = { twice, square, negate };
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    int (*ops[3])(int) = { twice, square, negate };
    ops[1] = k ? negate : square;
    int (*f)(int) = ops[1];
    for (int i = 0; i < 3; i++) printf("%d\n", ops[i](k));
    printf("%d %d\n", table[k % 3](10), f(k));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "static TABLE: [fn(i32) -> i32; 3] = [twice, square, negate];",
            "let mut ops: [fn(i32) -> i32; 3] = [twice, square, negate];",
            "ops[1] = if k != 0 { negate } else { square };",
            "let f: fn(i32) -> i32 = ops[1];",
            r#"println!("{}", ops[i as usize](k));"#,
            r#"println!("{} {}", TABLE[(k % 3) as usize](10), f(k));"#,
        ],
    );
}

#[test]
fn the_address_and_the_target_of_a_function_are_the_function() {
    let rust = check(
        "fn_address",
        &program(
            r#"
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    int (*f)(int) = &square;
    int (*g)(int) = *twice;
    if (k > 2) f = g;
    printf("%d %d %d\n", (*f)(k), (**g)(k), (&negate)(k));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut f: fn(i32) -> i32 = square;",
            "let g: fn(i32) -> i32 = twice;",
            r#"println!("{} {} {}", f(k), g(k), negate(k));"#,
        ],
    );
}

#[test]
fn records_numbers_and_enumerations_pass_by_value() {
    let rust = check(
        "fn_values",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct pt { int x, y; };
enum color { RED, GREEN, BLUE };
static int sum(struct pt p) { return p.x + p.y; }
static int diff(struct pt p) { return p.x - p.y; }
static double add(double a, enum color c) { return a + c; }
static double mul(double a, enum color c) { return a * c; }
static void hello(void) { printf("hello\n"); }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    int (*f)(struct pt) = k > 1 ? diff : sum;
    double (*g)(double, enum color) = k % 2 ? add : mul;
    void (*h)(void) = hello;
    struct pt p = { k, 3 };
    h();
    printf("%d %.2f\n", f(p), g(1.5, k % 3));
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "let f: fn(Pt) -> i32 = if k > 1 { diff } else { sum };",
            "let g: fn(f64, u32) -> f64 = if k % 2 != 0 { add } else { mul };",
            "let h: fn() = hello;",
            "h();",
        ],
    );
}

#[test]
fn the_callee_is_evaluated_before_its_arguments() {
    let rust = check(
        "fn_order",
        &program(
            r#"
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    int (*ops[3])(int) = { twice, square, negate };
    int i = k % 2;
    int r = ops[i++](k);
    printf("%d %d\n", r, i);
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(&rust, &["let r: i32 = ops[{"]);
}

#[test]
fn a_function_called_only_through_a_pointer_is_translated() {
    let rust = check(
        "fn_unnamed",
        &program(
            r#"
static int cube(int x) { return x * x * x; }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    int (*f)(int) = cube;
    printf("%d %d %d %d\n", f(k), twice(k), square(k), negate(k));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &["fn cube(x: i32) -> i32 {", "let f: fn(i32) -> i32 = cube;"],
    );
}

#[test]
fn a_pointer_to_a_function_that_may_be_null_is_an_option() {
    let rust = check(
        "fn_null",
        &program(
            r#"
static int (*hook)(int);
static void set(int k) { if (k) hook = negate; }
static void run(int (*f)(int), int x) { if (!f) return; printf("%d\n", f(x)); }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    int (*f)(int) = NULL;
    int (*g)(int) = 0;
    if (k == 1) f = twice;
    if (k == 2) f = square;
    if (k > 1) g = negate;
    if (f) printf("%d\n", f(k)); else printf("none\n");
    if (g != NULL) printf("%d\n", g(k));
    run(k ? twice : NULL, k);
    run(NULL, k);
    run(square, k);
    set(k);
    printf("%d\n", hook ? hook(k) : 0);
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "hook: Option<fn(i32) -> i32>,",
            "globals.hook = Some(negate);",
            "fn run(f: Option<fn(i32) -> i32>, x: i32) {",
            "let Some(f) = f else {",
            r#"println!("{}", f(x));"#,
            "let mut f: Option<fn(i32) -> i32> = None;",
            "f = Some(twice);",
            "if let Some(f) = f {",
            r#"println!("{}", f(k));"#,
            "if let Some(g) = g {",
            "run(if k != 0 { Some(twice) } else { None }, k);",
            "run(None, k);",
            "run(Some(square), k);",
        ],
    );
}

#[test]
fn a_call_through_a_pointer_that_may_be_null_unwraps_it() {
    let rust = check(
        "fn_null_call",
        &program(
            r#"
struct handler { int id; int (*on)(int); };
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    struct handler h = { 1 };
    int (*f)(int) = NULL;
    if (k >= 0) f = twice;
    if (k > 1) h.on = square;
    if (h.on) printf("%d\n", h.on(k)); else printf("%d none\n", h.id);
    printf("%d\n", f(k));
    return 0;
}
"#,
        ),
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "on: Option<fn(i32) -> i32>,",
            "Self { id: 0, on: None }",
            "let mut h = Handler { id: 1, on: None };",
            "h.on = Some(square);",
            r#"println!("{}", h.on.unwrap()(k));"#,
            r#"println!("{}", f.unwrap()(k));"#,
        ],
    );
}

#[test]
fn the_functions_of_one_type_take_the_globals_that_one_of_them_needs() {
    let rust = check(
        "fn_globals",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int total;
static int base = 10;
struct op { int (*run)(int); };
static void set(int k) { base = k; }
static int add(int x) { total += x; return total; }
static int peek(int x) { return total + x; }
static int pure(int x) { return x; }
static int reads(long x) { return base + (int)x; }
static long same(long x) { return x; }
static int through(struct op o, int x) { return o.run(x) * 2; }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    set(k * 3);
    struct op o = { k % 2 ? add : peek };
    int (*g)(int) = k > 1 ? pure : peek;
    int (*r)(long) = reads;
    long (*s)(long) = same;
    printf("%d %d %d\n", through(o, k), through(o, 1), g(2));
    printf("%d %ld %d\n", r(k), s(k), total);
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn add(globals: &mut Globals, x: i32) -> i32 {",
            "fn peek(globals: &mut Globals, x: i32) -> i32 {",
            "fn pure(_globals: &mut Globals, x: i32) -> i32 {",
            "fn reads(globals: &Globals, x: i64) -> i32 {",
            "fn same(x: i64) -> i64 {",
            "fn through(globals: &mut Globals, o: Op, x: i32) -> i32 {",
            "(o.run)(globals, x) * 2",
            "run: fn(&mut Globals, i32) -> i32,",
            "let g: fn(&mut Globals, i32) -> i32 = if k > 1 { pure } else { peek };",
            "let r: fn(&Globals, i64) -> i32 = reads;",
            "let s: fn(i64) -> i64 = same;",
            r#"println!("{} {} {}", r(globals, k as i64), s(k as i64), globals.total);"#,
        ],
    );
}

#[test]
fn a_pointer_without_a_prototype_takes_no_parameters_from_functions_without_them() {
    let rust = check(
        "fn_no_prototype_none",
        r#"
#include <stdio.h>
#include <stdlib.h>
typedef long (*source)();
static long low() { return 3; }
static long high(void) { return 40; }
struct gauge { long (*read)(); int scale; };
static long (*fallback)() = low;
static long sample(long (*from)()) { return from() + 1; }
static source pick(int k) { return k > 1 ? high : low; }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    struct gauge g = { high, 2 };
    long (*both[2])() = { low, high };
    source s = pick(k);
    printf("%ld %ld %ld %ld %ld\n", g.read() * g.scale, fallback(), sample(low), s(), both[k % 2]());
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "const FALLBACK: fn() -> i64 = low;",
            "read: fn() -> i64,",
            "fn sample(from: fn() -> i64) -> i64 {",
            "fn pick(k: i32) -> fn() -> i64 {",
            "let both: [fn() -> i64; 2] = [low, high];",
            "let s: fn() -> i64 = pick(k);",
        ],
    );
}

#[test]
fn a_pointer_without_a_prototype_that_only_holds_null_is_an_option() {
    let rust = check(
        "fn_no_prototype_null",
        "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char **argv) {\n    int (*hook)() = 0;\n    if (argc > 3 || hook) printf(\"hooked\\n\");\n    else printf(\"plain %d\\n\", atoi(argv[1]));\n    return 0;\n}\n",
        RUNS,
        "",
    );
    assert_lines(&rust, &["let hook: Option<fn() -> i32> = None;"]);
}

#[test]
fn a_pointer_without_a_prototype_takes_the_parameters_of_its_functions() {
    let rust = check(
        "fn_no_prototype_params",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct span { int from; int to; };
static double scaled(struct span s, double by, int shift) { return (s.to - s.from) * by + shift; }
static double offset(struct span s, double by, int shift) { return s.from + by - shift; }
int main(int argc, char **argv) {
    int k = atoi(argv[1]);
    struct span s = { k, 2 * k + 1 };
    double (*measure)() = k % 2 ? scaled : offset;
    printf("%.2f\n", measure(s, 0.5, k));
    measure = scaled;
    printf("%.2f\n", measure(s, 1.25, 3));
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut measure: fn(Span, f64, i32) -> f64 = if k % 2 != 0 {",
            "println!(\"{:.2}\", measure(s, 0.5, k));",
            "measure = scaled;",
        ],
    );
}

#[test]
fn what_no_rule_holds_for_a_pointer_to_a_function_is_refused() {
    let refused = |name, main: &str| translate(name, &program(main)).unwrap_err();
    let cases = [
        (
            "fn_pointer_param",
            "static int first(const char *s) { return s[0]; }\nint main(void) { int (*f)(const char *) = first; return f(\"a\"); }\n",
            "unsupported: a pointer to a function that takes a pointer, at line 8",
        ),
        (
            "fn_compare",
            "int main(int argc, char **argv) { int (*f)(int) = argc ? twice : square; return f == twice; }\n",
            "unsupported: a comparison of two functions, at line 7",
        ),
        (
            "fn_cast",
            "int main(int argc, char **argv) { long n = (long)twice; return n == 0; }\n",
            "unsupported: a conversion with no rule, at line 7",
        ),
        (
            "fn_cast_type",
            "int main(int argc, char **argv) { int (*f)(long) = (int (*)(long))twice; return f(1); }\n",
            "unsupported: a cast of a pointer to a function, at line 7",
        ),
        (
            "fn_void",
            "int main(int argc, char **argv) { void *p = (void *)twice; return p == 0; }\n",
            "unsupported: a function kept in a `void` pointer: no rule yet, at line 7",
        ),
        (
            "fn_no_prototype_several",
            "static int add(int x, int y) { return x + y; }\nint main(int argc, char **argv) { int (*f)() = twice; if (argc > 1) f = add; return f(1); }\n",
            "unsupported: a pointer to a function without a prototype that holds functions of several types, at line 8",
        ),
        (
            "fn_no_prototype_count",
            "int main(int argc, char **argv) { int (*f)() = twice; return f(1, 2); }\n",
            "unsupported: a call through a pointer to a function without a prototype with arguments its functions do not take, at line 7",
        ),
        (
            "fn_no_prototype_other_type",
            "int main(int argc, char **argv) { int (*f)() = twice; return f(1L); }\n",
            "unsupported: a call through a pointer to a function without a prototype with arguments its functions do not take, at line 7",
        ),
        (
            "fn_no_prototype_promoted",
            "static int up(short x) { return x + 1; }\nint main(int argc, char **argv) { short s = 2; int (*f)() = (int (*)())up; return f(s); }\n",
            "unsupported: a call through a pointer to a function without a prototype with arguments its functions do not take, at line 8",
        ),
        (
            "fn_no_prototype_unreached",
            "static int (*hook)();\nint main(int argc, char **argv) { return hook ? hook(argc) : 0; }\n",
            "unsupported: a call through a pointer to a function without a prototype that no function reaches, at line 8",
        ),
        (
            "fn_variadic",
            "int main(int argc, char **argv) { int (*f)(const char *, ...) = printf; f(\"x\"); return 0; }\n",
            "unsupported: a pointer to a variadic function, at line 7",
        ),
        (
            "fn_library",
            "int main(int argc, char **argv) { int (*f)(int) = abs; return f(-argc); }\n",
            "unsupported: the address of the library's `abs`: no rule yet, at line 7",
        ),
        (
            "fn_handed_to_library",
            "static void done(void) { printf(\"done\\n\"); }\nint main(void) { atexit(done); return 0; }\n",
            "unsupported: this call of the library's `atexit`: no rule yet, at line 8",
        ),
        (
            "fn_pointer_to_pointer",
            "static void pick(int (**out)(int)) { *out = twice; }\nint main(void) { int (*f)(int) = square; pick(&f); return f(1); }\n",
            "unsupported: a pointer to a pointer to a function, at line 7",
        ),
        (
            "fn_sizeof",
            "int main(int argc, char **argv) { int (*f)(int) = twice; return f(1) + (int)sizeof f; }\n",
            "unsupported: `sizeof` of a pointer to a function, at line 7",
        ),
        (
            "fn_array_null",
            "int main(int argc, char **argv) { int (*ops[3])(int) = { twice, square }; return ops[argc](1); }\n",
            "unsupported: a pointer to a function that starts at NULL, at line 7",
        ),
        (
            "fn_array_store_null",
            "int main(int argc, char **argv) { int (*ops[2])(int) = { twice, square }; ops[1] = 0; return ops[argc](1); }\n",
            "unsupported: a pointer to a function that may be NULL in an array, at line 7",
        ),
        (
            "fn_array_store_nullable",
            "static int (*find(int k))(int) { return k ? twice : 0; }\nint main(int argc, char **argv) { int (*ops[2])(int) = { twice, square }; ops[1] = find(argc - 1); return ops[1] ? ops[1](1) : 0; }\n",
            "unsupported: a pointer to a function that may be NULL in an array, at line 8",
        ),
        (
            "fn_array_init_nullable",
            "int main(int argc, char **argv) { int (*p)(int) = argc > 1 ? negate : 0; int (*ops[2])(int) = { twice, p }; return ops[1] ? ops[1](1) : 0; }\n",
            "unsupported: a pointer to a function that may be NULL in an array, at line 7",
        ),
        (
            "fn_percent_p",
            "int main(int argc, char **argv) { int (*f)(int) = twice; printf(\"%p\\n\", (void *)f); return 0; }\n",
            "unsupported: the `printf` conversion `%p`: no rule yet, at line 7",
        ),
        (
            "fn_globals_nested",
            "static int total;\nstatic int add(int x) { total += x; return total; }\nint main(int argc, char **argv) { int (*f)(int) = add; return f(f(argc)); }\n",
            "unsupported: a call through a pointer whose arguments use the globals that it changes, at line 9",
        ),
    ];
    for (name, main, refusal) in cases {
        assert_eq!(refused(name, main), refusal, "{name}");
    }
}

/// A record that owns another of its own type, never NULL, has no zero to
/// start with: the translation refuses it instead of writing one without
/// an end.
#[test]
fn a_record_that_owns_itself_without_null_is_refused() {
    let c = "#include <stdio.h>\n#include <stdlib.h>\nstruct node { struct node *next; int value; };\nint main(void) {\n    struct node *ring = malloc(sizeof(struct node));\n    ring->value = 3;\n    ring->next = ring;\n    printf(\"%d\\n\", ring->next->value);\n    return 0;\n}\n";
    assert_eq!(
        translate("ring", c).unwrap_err(),
        "unsupported: a record that owns a record of its own type that is never NULL"
    );
}
