use crate::common::{assert_lines, check};

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
