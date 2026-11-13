use crate::common::{assert_lines, check, translate};

#[test]
fn records_and_arrays_hold_values() {
    let rust = check(
        "records",
        r#"
#include <stdio.h>
#include <stdlib.h>
typedef struct { int x; int y; } point_t;
struct line { point_t from, to; double weight; int marks[3]; };
point_t shifted(point_t p, int by) { p.x += by; return p; }
int length2(struct line l) {
    int dx = l.to.x - l.from.x, dy = l.to.y - l.from.y;
    return dx * dx + dy * dy;
}
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    point_t origin = {0};
    struct line l = {{1, 2}, {.x = n, .y = 4}, 0.5, {7}};
    point_t copy = l.to;
    copy.y = 99;
    l.from = shifted(origin, n);
    int table[2][3] = {{1, 2, 3}, {4, 5, 6}};
    unsigned char bytes[4] = {1, 2};
    long total = 0;
    for (int i = 0; i < 2; i++)
        for (int j = 0; j < 3; j++)
            total += table[i][j] * l.marks[j];
    l.marks[n % 3] = bytes[1];
    printf("%d %d %d %ld %d %.1f %lu\n", length2(l), copy.y, l.to.y, total, l.marks[n % 3], l.weight,
           sizeof(table) / sizeof(table[0]));
    return 0;
}
"#,
        &[&["0"], &["5"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "#[derive(Clone, Copy, Default)]",
            "struct PointT {",
            "fn shifted(mut p: PointT, by: i32) -> PointT {",
            "let origin: PointT = PointT { x: 0, y: 0 };",
            "let table: [[i32; 3]; 2] = [[1, 2, 3], [4, 5, 6]];",
        ],
    );
}

#[test]
fn an_initialiser_of_zeros_fills_an_array_of_any_length() {
    let rust = check(
        "zeros",
        r#"
#include <stdio.h>
struct pair { int a; double b; };
int main(void) {
    int counts[40] = {0};
    double sums[100] = {0.0};
    struct pair pairs[50] = {0};
    unsigned char few[3] = {0, 0};
    counts[39] = 7;
    printf("%d %d %.1f %d %.1f %d\n", counts[0], counts[39], sums[99], pairs[49].a, pairs[49].b, few[2]);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut counts: [i32; 40] = [0; 40];",
            "let sums: [f64; 100] = [0.0; 100];",
            "let few: [u8; 3] = [0; 3];",
        ],
    );
}

#[test]
fn an_array_of_a_computed_length_is_a_vec() {
    let rust = check(
        "vla",
        r#"
#include <stdio.h>
#include <stdlib.h>
long total(const long *values, int count) {
    long sum = 0;
    for (int i = 0; i < count; i++) sum += values[i];
    return sum;
}
int main(int argc, char *argv[]) {
    int count = atoi(argv[1]);
    if (count <= 0) return 1;
    long squares[count];
    unsigned char flags[count];
    for (int i = 0; i < count; i++) {
        squares[i] = (long)i * i;
        flags[i] = i % 2;
    }
    count = 1;
    printf("%ld %d %ld\n", total(squares, atoi(argv[1])), flags[atoi(argv[1]) - 1], total(squares, count));
    return 0;
}
"#,
        &[&["1"], &["2"], &["9"], &["0"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut squares: Vec<i64> = vec![0; count as usize];",
            "let mut flags: Vec<u8> = vec![0; count as usize];",
            "total(&squares, parse_int(&args[1]) as i32),",
        ],
    );
    assert_eq!(
        translate(
            "vla-expression",
            "int main(int argc, char *argv[]) {\n    int values[argc + 1];\n    values[0] = 0;\n    return values[0];\n}\n"
        )
        .unwrap_err(),
        "unsupported: the type `int[argc + 1]`, at line 2"
    );
    assert_eq!(
        translate(
            "vla-shadow",
            "int main(int argc, char *argv[]) {\n    int n = 2;\n    { int n = 3; argc += n; }\n    int values[n];\n    values[0] = 0;\n    return values[0];\n}\n"
        )
        .unwrap_err(),
        "unsupported: an array whose length is not a constant or one variable, at line 4"
    );
}

#[test]
fn a_bit_field_keeps_the_bits_it_has() {
    let rust = check(
        "bit-fields",
        r#"
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
struct flags {
    unsigned int level : 3;
    int offset : 4;
    bool on : 1;
    unsigned int whole : 32;
    int plain;
};
void show(const struct flags *flags) {
    printf("%u %d %d %u %d %d\n", flags->level, flags->offset, flags->on, flags->whole,
           flags->plain, flags->level + flags->offset);
}
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct flags flags = {.level = n, .offset = n, .on = n > 2, .whole = n, .plain = n};
    struct flags zero = {0};
    show(&flags);
    show(&zero);
    flags.level = n * 3;
    flags.offset = -n;
    flags.on = n & 1;
    zero.level = 9;
    zero.offset = 8;
    show(&flags);
    show(&zero);
    return 0;
}
"#,
        &[&["0"], &["1"], &["5"], &["7"], &["8"], &["-9"], &["100"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "level: u32,",
            "offset: i32,",
            "on: bool,",
            "level: n as u32 & 7,",
            "offset: n << 28 >> 28,",
            "flags.level = (n * 3) as u32 & 7;",
            "flags.offset = -n << 28 >> 28;",
        ],
    );
    assert_eq!(
        translate(
            "bit-field-update",
            "struct s { unsigned a : 3; };\nint main(void) {\n    struct s s = {1};\n    s.a += 7;\n    return s.a;\n}\n"
        )
        .unwrap_err(),
        "unsupported: an update of a bit-field in place: no rule yet, at line 4"
    );
}

#[test]
fn a_bit_field_of_a_number_holds_its_bits() {
    let rust = check(
        "bit-field-number",
        r#"
#include <stdio.h>
struct flags {
    unsigned on : 1;
    int level : 3;
    unsigned wide : 31;
};
int main(void) {
    struct flags flags = {1, 3, 2147483647};
    printf("%u %d %u\n", flags.on, flags.level, flags.wide);
    flags.on = 1;
    flags.level = 7;
    flags.wide = 7;
    printf("%u %d %u\n", flags.on, flags.level, flags.wide);
    flags.on = 2;
    flags.level = 12;
    printf("%u %d %u\n", flags.on, flags.level, flags.wide);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "on: 1,",
            "wide: 2147483647,",
            "flags.on = 1;",
            "flags.level = -1;",
            "flags.on = 0;",
            "flags.level = -4;",
        ],
    );
}
