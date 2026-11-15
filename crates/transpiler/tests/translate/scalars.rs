use crate::common::{assert_lines, check, translate};

#[test]
fn arithmetic_follows_c_types() {
    let rust = check(
        "arithmetic",
        r#"
#include <stdio.h>
#include <stdlib.h>
unsigned wrap(unsigned a, unsigned b) { return a * b + 7u - b; }
long mix(int a, long b, unsigned char c) { return a + b * c; }
double half(int n) { return n / 2 + n / 2.0; }
int main(int argc, char *argv[]) {
    if (argc != 2) return 3;
    int n = atoi(argv[1]);
    unsigned char small = 250;
    small += n;
    char letter = 'a';
    letter++;
    short narrow = (short)(n * 1000);
    printf("%u %ld %.3f\n", wrap(4000000000u, n), mix(n, 1L << 40, small), half(n));
    printf("%d %c %d %x %5d|%-5d|%05d\n", small, letter, narrow, n ^ 0xff, n, n, n);
    printf("%d %d %d\n", n > 3 && n < 100, !n, (n & 1) ? n << 2 : ~n);
    return n % 256;
}
"#,
        &[&["7"], &["0"], &["-12"], &["300"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn wrap(a: u32, b: u32) -> u32 {",
            "a.wrapping_mul(b).wrapping_add(7).wrapping_sub(b)",
            "small = small.wrapping_add(n as u8);",
        ],
    );
}

#[test]
fn loops_and_switches_become_rust_control_flow() {
    let rust = check(
        "control",
        r#"
#include <stdio.h>
#include <stdlib.h>
enum kind { SMALL, MEDIUM = 5, LARGE };
int classify(int n) {
    switch (n) {
        case 0: return SMALL;
        case 1:
        case 2: { int twice = n * 2; return twice + MEDIUM; }
        default: break;
    }
    return LARGE;
}
int main(int argc, char *argv[]) {
    int limit = (int)strtol(argv[1], NULL, 10);
    int sum = 0;
    for (int i = 0; i < limit; i++) {
        if (i % 2) continue;
        sum += classify(i);
    }
    int i = 0;
    while (1) {
        if (i * i > limit) break;
        i++;
    }
    do { sum -= 1; } while (sum > 100);
    for (int j = limit; j > 0; j /= 2) sum += j;
    printf("%d %d\n", sum, i);
    return 0;
}
"#,
        &[&["0"], &["9"], &["1000"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "const MEDIUM: i32 = 5;",
            "for i in 0..limit {",
            "match n {",
            "1 | 2 => {",
            "loop {",
            "fn main() {",
        ],
    );
}

/// Cases that an earlier version translated to Rust that ran differently.
#[test]
fn values_and_names_that_are_easy_to_get_wrong() {
    let rust = check(
        "corners",
        r#"
#include <stdio.h>
enum level { LOW = (unsigned long)1, HIGH };
enum sign { MINUS = -1, PLUS = 1 };
int main(void) {
    int n = 0;
    unsigned top = 4294967295u;
    for (unsigned i = top; i <= top; i++) {
        printf("%u\n", i);
        if (++n == 2) break;
    }
    int total = 0;
    for (int i = 1; i <= 3; i++) total += i;
    int before = 0;
    int a[2] = {1, 2};
    int x = a[before]++;
    int type = 1;
    int type_ = 2;
    enum sign s = MINUS;
    printf("%d %d %d %d %d %d\n", total, x, a[0], a[1], type, type_);
    printf("%d %d %d\n", LOW, HIGH, s < 0);
    printf("%.20f %.20f\n", 0.1f, (double)0.5f);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "for i in 1..=3 {",
            "while i <= top {",
            "let type__: i32 = 1;",
        ],
    );
}

/// C89 declares at the top of a block; the translation declares where the
/// first value is known.
#[test]
fn a_local_is_declared_where_it_gets_its_first_value() {
    let rust = check(
        "join",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    int limit, i, j, total, last;
    const char *label;
    limit = atoi(argv[1]);
    total = 0;
    label = "total";
    for (i = 0; i < limit; i++) {
        int square;
        square = i * i;
        total += square;
    }
    for (j = 0; j < limit; j++) {
        if (j * j > limit) break;
    }
    last = 0;
    if (limit > 3) last = j;
    printf("%s %d %d\n", label, total, last);
    return 0;
}
"#,
        &[&["0"], &["5"], &["30"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let limit = parse_int(&args[1]) as i32;",
            "let mut total: i32 = 0;",
            "let label: &str = \"total\";",
            "for i in 0..limit {",
            "let square: i32 = i * i;",
            "let mut j: i32 = 0;",
            "let mut last: i32 = 0;",
        ],
    );
}

#[test]
fn strtoull_reads_what_c_reads() {
    let rust = check(
        "strtoull",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    unsigned long long wide = strtoull(argv[1], NULL, 10);
    unsigned int narrow = (unsigned int)strtoul(argv[1], NULL, 10);
    printf("%llu %u\n", wide, narrow);
    return 0;
}
"#,
        &[
            &["0"],
            &["4000000000"],
            &["  +18446744073709551615"],
            &["18446744073709551616"],
            &["99999999999999999999999"],
            &["-1"],
            &["-18446744073709551615"],
            &["-99999999999999999999999"],
            &["12abc"],
            &["abc"],
            &[""],
            &["- 5"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let wide: u64 = parse_uint(&args[1]);",
            "let narrow = parse_uint(&args[1]) as u32;",
        ],
    );
}

#[test]
fn strtod_reads_what_c_reads() {
    let rust = check(
        "strtod",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    double wide = strtod(argv[1], NULL);
    float narrow = strtof(argv[1], NULL);
    double plain = atof(argv[1]);
    printf("%.17f %.12f %.3f %d\n", wide, narrow, plain, wide > 1.0);
    return 0;
}
"#,
        &[
            &["0"],
            &["1.5"],
            &["  -2.25e2xyz"],
            &["+.5"],
            &["5."],
            &["."],
            &["-"],
            &["1e"],
            &["1e+"],
            &["1e-3"],
            &["0.1"],
            &["16777217"],
            &["1.00000017881393432617187499"],
            &["3.4028236e38"],
            &["1e400"],
            &["-1e-400"],
            &["inf"],
            &["-INFINITY and more"],
            &["infinit"],
            &["abc"],
            &[""],
            &["12abc"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let wide: f64 = parse_float::<f64>(&args[1]);",
            "let narrow: f32 = parse_float::<f32>(&args[1]);",
            "fn parse_float<T: std::str::FromStr + Default + Nan>(text: &str) -> T {",
        ],
    );
}

#[test]
fn printf_e_and_g_print_what_c_prints() {
    let rust = check(
        "format-g",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    double wide = strtod(argv[1], NULL);
    float narrow = strtof(argv[1], NULL);
    printf("%g|%.9g|%.17g|%.0g|%.1g|%12.4g|%-12.3g|\n", wide, narrow, wide, wide, wide, wide, wide);
    printf("%e|%.0e|%.3e|%14.2e|%-14.10e|\n", wide, wide, narrow, wide, wide);
    return 0;
}
"#,
        &[
            &["0"],
            &["-0.0"],
            &["1"],
            &["100000"],
            &["1000000"],
            &["999999.5"],
            &["0.0001"],
            &["0.00001"],
            &["0.00009999995"],
            &["123456789.125"],
            &["-1.5e300"],
            &["2.5e-300"],
            &["0.1"],
            &["9.5"],
            &["0.95"],
            &["16777217"],
            &["3.14159265358979323846"],
            &["1e400"],
            &["-inf"],
            &["nan"],
            &["5e-324"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "format_g(wide, 6),",
            "format_g(narrow as f64, 9),",
            "format_e(wide, 0),",
            "fn format_g(value: f64, precision: usize) -> String {",
        ],
    );
}

#[test]
fn names_follow_rust_conventions() {
    let rust = check(
        "snake",
        r#"
#include <stdio.h>
struct lineInfo { int lineCount; int MaxWidth; };
static int totalLines = 0;
static const int maxLines = 3;
int countLine(struct lineInfo *lineInfo, int lineWidth) {
    int X = lineWidth, x = 1, line_width = 2, _unused_Value = 3;
    lineInfo->lineCount += x;
    if (X > lineInfo->MaxWidth) lineInfo->MaxWidth = X;
    totalLines++;
    return line_width + _unused_Value + (totalLines > maxLines);
}
int HTTPStatus(int goodG2B) { return goodG2B + 200; }
int main(void) {
    struct lineInfo info = {0, 0};
    int count_line = countLine(&info, 7) + countLine(&info, 9);
    printf("%d %d %d %d %d\n", info.lineCount, info.MaxWidth, totalLines, count_line, HTTPStatus(4));
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "const MAX_LINES: i32 = 3;",
            "struct LineInfo {",
            "line_count: i32,",
            "max_width: i32,",
            "total_lines: i32,",
            "fn count_line(globals: &mut Globals, line_info: &mut LineInfo, line_width_: i32) -> i32 {",
            "let x_: i32 = line_width_;",
            "let line_width: i32 = 2;",
            "let _unused_value: i32 = 3;",
            "fn httpstatus(good_g2_b: i32) -> i32 {",
            "let count_line_: i32 = count_line(globals, &mut info, 7)",
        ],
    );
}

#[test]
fn printf_f_in_capitals_is_refused() {
    // `%F` prints an infinity as `INF`.
    assert_eq!(
        translate(
            "printf-capital-f",
            "#include <stdio.h>\n#include <stdlib.h>\nint main(void) {\n    printf(\"%F\\n\", strtod(\"inf\", 0));\n    return 0;\n}\n"
        )
        .unwrap_err(),
        "unsupported: the `printf` conversion `%F`: no rule yet, at line 4"
    );
}

#[test]
fn a_local_named_underscore_gets_a_name() {
    let rust = check(
        "underscore",
        "#include <stdio.h>\nint main(void) {\n    int _ = 3;\n    printf(\"%d\\n\", _ + 1);\n    return 0;\n}\n",
        &[&[]],
        "",
    );
    assert_lines(&rust, &["let __: i32 = 3;"]);
}

/// Two C names that Rust's conventions would give one Rust name stay two.
#[test]
fn names_that_rust_would_merge_stay_apart() {
    let rust = check(
        "merged",
        r#"
#include <stdio.h>
struct s { int fooBar; int foo_bar; int FooBar; };
enum { foo_baz = 1 };
int f(void) { enum { X = 1 }; return X; }
int g(void) { enum { X = 2 }; return X; }
int main(void) {
    struct s s = {1, 2, 3};
    int fooBaz = 4;
    printf("%d %d %d %d %d %d %d\n", s.fooBar, s.foo_bar, s.FooBar, fooBaz, foo_baz, f(), g());
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "const FOO_BAZ: i32 = 1;",
            "const X: i32 = 1;",
            "const X_: i32 = 2;",
            "foo_bar_: i32,",
            "foo_bar: i32,",
            "foo_bar__: i32,",
            "let foo_baz: i32 = 4;",
        ],
    );
}

#[test]
fn a_number_compared_with_itself_is_tested_for_nan() {
    let rust = check(
        "nan",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    double d = strtod(argv[1], NULL);
    float f = (float)d;
    struct pair { double a; double b; } p = {d, 1.0};
    double xs[2] = {0.5, d};
    printf("%d %d %d\n", d != d, f == f, d == d ? 1 : 0);
    printf("%d %d %d\n", p.a != p.a, xs[argc - 1] == xs[argc - 1], p.a != p.b);
    return 0;
}
"#,
        &[&["1.5"], &["nan"], &["inf"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "i32::from(d.is_nan()),",
            "i32::from(!f.is_nan()),",
            "i32::from(p.a.is_nan()),",
            "i32::from(!xs[(args.len() as i32 - 1) as usize].is_nan()),",
            "i32::from(p.a != p.b),",
        ],
    );
}

#[test]
fn the_same_value_on_both_sides_is_refused_where_the_lints_refuse_it() {
    // Clippy's `eq_op` is an error, so these would not compile.
    let refused = |name, value: &str| {
        translate(
            name,
            &format!("int main(int argc, char **argv) {{\n    return {value};\n}}\n"),
        )
        .unwrap_err()
    };
    assert_eq!(
        refused("self-equal", "argc == argc"),
        "unsupported: a comparison of a value with itself: no rule yet, at line 2"
    );
    assert_eq!(
        refused("self-minus", "(argc + 1) - (argc + 1)"),
        "unsupported: the same value on both sides of `-`: no rule yet, at line 2"
    );
}

#[test]
fn a_literal_near_a_named_constant_keeps_its_value() {
    // Clippy takes `2.718281828` for `std::f64::consts::E` written out and
    // refuses it; the program means the number it wrote.
    let rust = check(
        "near-constant",
        r#"
#include <stdio.h>
int main(void) {
    double e = 2.718281828, pi = 3.14, root = 1.41421, half = 0.5, tau = -6.2832;
    double exact = 3.141592653589793, golden = 1.6180339, product = 2.0 * 0.6931;
    float small = 0.3183f;
    printf("%.17g %.17g %.17g %.17g %.17g\n", e, pi, root, half, tau);
    printf("%.17g %.17g %.17g %.9g\n", exact, golden, product, small);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let e: f64 = 2718281828.0 / 1e9;",
            "let pi: f64 = 314.0 / 1e2;",
            "let root: f64 = 141421.0 / 1e5;",
            "let half: f64 = 0.5;",
            "let tau: f64 = -(62832.0 / 1e4);",
            "let exact: f64 = 3141592653589793.0 / 1e15;",
            "let product: f64 = 2.0 * (6931.0 / 1e4);",
        ],
    );
}

#[test]
fn a_break_in_the_middle_of_a_case_leaves_the_match() {
    let rust = check(
        "case-break",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int pick(int mode, int x) {
    int r = x;
    switch (mode) {
    case 1:
        if (x > 0) break;
        return -1;
    case 2:
        r = x * 2;
        switch (x) {
        case 1:
            if (mode == 2) break;
            r = 50;
            break;
        default:
            r += 1;
        }
        if (r > 3) break;
        r = 0;
        break;
    case 3:
        for (int i = 0; i < 5; i++) {
            if (i == x) break;
            r += i;
        }
        break;
    default:
        return 7;
    }
    return r + 1;
}
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    int mode = atoi(argv[1]);
    printf("%d %d %d %d\n", pick(mode, 1), pick(mode, 2), pick(mode, -3), pick(mode, 9));
    return 0;
}
"#,
        &[&["1"], &["2"], &["3"], &["4"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "'switch: {",
            "break 'switch;",
            "'switch_2: {",
            "break 'switch_2;",
            "break;",
        ],
    );
    // A `continue` would have to name its loop to leave the block.
    assert_eq!(
        translate(
            "case-break-continue",
            "int main(int argc, char **argv) {\n    int n = 0;\n    while (n < 5) {\n        n++;\n        switch (n) {\n        case 1:\n            if (argc) break;\n            n++;\n            break;\n        case 2:\n            continue;\n        }\n    }\n    return n;\n}\n"
        )
        .unwrap_err(),
        "unsupported: `continue` in a `switch` with a `break` in the middle of a `case`: no rule yet, at line 5"
    );
}

#[test]
fn a_function_that_returns_nothing_ends_without_return() {
    let rust = check(
        "last_return",
        r#"
#include <stdio.h>
#include <stdlib.h>
void show(int n) {
    if (n < 0) {
        puts("negative");
        return;
    }
    printf("%d\n", n);
    return;
}
int main(int argc, char *argv[]) {
    show(atoi(argv[1]));
    return 0;
}
"#,
        &[&["3"], &["-2"]],
        "",
    );
    // Only the `return` in the branch is left.
    assert_eq!(
        rust.lines().filter(|line| line.trim() == "return;").count(),
        1,
        "{rust}"
    );
}

#[test]
fn an_update_of_a_place_by_itself_is_compound() {
    let rust = check(
        "compound",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct box { int count; double weight; };
int twice(int n) { return 2 * n; }
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    int x = n;
    int a[3] = {1, 2, 3};
    struct box b = {n, 0.5};
    unsigned u = n;
    char c = 'a';
    x = x * n;
    a[n % 3] = a[n % 3] + x;
    b.count = b.count - 1;
    b.weight = b.weight * 3.0;
    u = u * 7u;
    c = c + 1;
    x = x + twice(n);
    printf("%d %d %d %.1f %u %c\n", x, a[n % 3], b.count, b.weight, u, c);
    return 0;
}
"#,
        &[&["3"], &["4"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "x *= n;",
            "a[(n % 3) as usize] += x;",
            "b.count -= 1;",
            "b.weight *= 3.0;",
            "u = u.wrapping_mul(7);",
            "c = (c as i8 as i32 + 1) as u8;",
            "x = x + twice(n);",
        ],
    );
}

#[test]
fn a_conversion_and_back_is_the_value_itself() {
    let rust = check(
        "round_trip",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdbool.h>
struct flags { unsigned level : 3; };
void show(bool on, int n) { printf("%d %d\n", on, n); }
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct flags f = {0};
    f.level = n;
    unsigned u = n;
    char c = n;
    printf("%x %u %u %d\n", (unsigned)(int)u, (unsigned)(long)u, (unsigned)(short)u, (char)(int)c);
    printf("%u\n", f.level);
    show(!!n, !!n);
    return 0;
}
"#,
        &[&["3"], &["-70000"], &["200"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "println!(\"{:x} {} {} {}\", u, u, u as i16 as u32, c as i8 as i32 as u8 as i8 as i32);",
            "println!(\"{}\", f.level);",
            "show(n != 0, i32::from(n != 0));",
        ],
    );
}

#[test]
fn div_is_a_record_of_the_quotient_and_the_remainder() {
    let rust = check(
        "div",
        r#"
#include <stdio.h>
#include <stdlib.h>
static ldiv_t halves(long n) { return ldiv(n, 2); }
int main(int argc, char *argv[]) {
    if (argc != 3) return 2;
    int x = atoi(argv[1]), y = atoi(argv[2]);
    div_t result = div(x, y);
    div_t copy = result;
    copy.rem += 1;
    long big = atol(argv[1]);
    ldiv_t wide = ldiv(big * 100000L, 7L - y);
    lldiv_t wider = lldiv(-9000000000LL, (long long)y);
    printf("%d %d %d\n", result.quot, result.rem, copy.rem);
    ldiv_t half = halves(x);
    div_t third = div(x + 1, 3);
    printf("%ld %ld %ld\n", wide.quot, wide.rem, half.rem);
    printf("%lld %lld %d\n", wider.quot, wider.rem, third.quot);
    return 0;
}
"#,
        &[
            &["7", "2"],
            &["-7", "2"],
            &["7", "-2"],
            &["-7", "-2"],
            &["0", "5"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "struct DivT {",
            "quot: i32,",
            "struct LdivT {",
            "let result = DivT { quot: x / y, rem: x % y };",
        ],
    );
    assert_eq!(
        translate(
            "div_effect",
            "#include <stdio.h>\n#include <stdlib.h>\nint main(void) {\n    int n = 7;\n    div_t d = div(n++, 2);\n    printf(\"%d\\n\", d.quot + n);\n    return 0;\n}\n"
        )
        .unwrap_err(),
        "unsupported: `div` of a value with a side effect: no rule yet, at line 5"
    );
}

#[test]
fn rand_reads_the_numbers_of_a_random_that_its_function_holds() {
    let rust = check(
        "rand_seeded",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    unsigned seed = (unsigned)strtoul(argv[1], NULL, 10);
    srand(seed);
    int first = rand();
    long sum = 0;
    for (int i = 0; i < 1000; i++) sum += rand() % 1000;
    printf("%d %ld %d\n", first, sum, rand() / (RAND_MAX / 6 + 1));
    srand(seed + 1);
    printf("%d\n", rand());
    return 0;
}
"#,
        &[&["0"], &["1"], &["42"], &["2147483648"], &["4294967295"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut random = Random::new(seed);",
            "let first: i32 = random.next();",
            "random = Random::new(seed.wrapping_add(1));",
        ],
    );
    // Without `srand`, and with one that only some paths reach, the
    // numbers start as those of the seed 1.
    let rust = check(
        "rand_unseeded",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    printf("%d\n", rand());
    if (argc > 1) srand(7);
    printf("%d %d\n", rand(), rand() % 6);
    return 0;
}
"#,
        &[&[], &["x"]],
        "",
    );
    assert_lines(&rust, &["let mut random = Random::new(1);"]);
    let refused = |name: &str, body: &str| {
        let c = format!("#include <stdio.h>\n#include <stdlib.h>\n{body}");
        translate(name, &c).unwrap_err()
    };
    assert_eq!(
        refused(
            "rand_two",
            "int roll(void) { return rand() % 6; }\nint main(void) {\n    srand(3);\n    printf(\"%d\\n\", roll());\n    return 0;\n}\n"
        ),
        "unsupported: `rand` in more than one function: no rule yet, at line 3"
    );
    assert_eq!(
        refused(
            "rand_none",
            "int main(void) {\n    srand(3);\n    return 0;\n}\n"
        ),
        "unsupported: `srand` in a program without `rand`, at line 4"
    );
}
