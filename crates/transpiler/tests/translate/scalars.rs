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
            "let limit: i32 = parse_int(&args[1]) as i32;",
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
            "let narrow: u32 = parse_uint(&args[1]) as u32;",
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
            "fn parse_float<T: std::str::FromStr + Default>(text: &str) -> T {",
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
