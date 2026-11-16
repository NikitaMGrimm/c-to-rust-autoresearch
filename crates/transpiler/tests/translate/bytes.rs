use crate::common::{assert_lines, check, translate};

/// R69: a number of 2, 4 or 8 bytes read or stored through a pointer to
/// bytes cast to a pointer to it is made from the bytes or stored as them.
#[test]
fn a_number_in_bytes_is_made_from_them() {
    let rust = check(
        "number_in_bytes",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
struct frame { int tag; uint8_t body[16]; };
static long first_word(const unsigned char *raw, int count) {
    if (raw == NULL || count < 4) return -1;
    const int *word = (const int *)raw;
    return *word + word[1];
}
static double scale(char *cells) { return *(double *)(cells + 8) * 2; }
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    unsigned char buf[16] = {0};
    *(short *)buf = (short)n;
    *(unsigned *)&buf[4] = (unsigned)n * 3u;
    struct frame f = {7, {0}};
    *(long long *)(f.body + 8) = -(long long)n;
    char *cells = malloc(16);
    *(double *)(cells + 8) = n / 4.0;
    *(float *)cells = 1.5f;
    unsigned char *heap = malloc(12);
    for (int i = 0; i < 3; i++) ((uint32_t *)heap)[i] = (uint32_t)(n + i);
    printf("%d %u %lld %.2f %.2f %ld %u\n", *(short *)buf, *(unsigned *)(buf + 4),
           *(long long *)&f.body[8], scale(cells), *(float *)cells,
           first_word(buf, 16), ((uint32_t *)heap)[2]);
    free(cells);
    free(heap);
    return 0;
}
"#,
        &[&["5"], &["-70000"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn first_word(raw: &[u8], count: i32) -> i64 {",
            "(i32::from_ne_bytes(raw[..4].try_into().unwrap())",
            "+ i32::from_ne_bytes(raw[4..][..4].try_into().unwrap())) as i64",
            "f64::from_ne_bytes(cells[8..][..8].try_into().unwrap()) * 2.0",
            "buf[..2].copy_from_slice(&i16::to_ne_bytes(n as i16));",
            "buf[4..][..4].copy_from_slice(&u32::to_ne_bytes((n as u32).wrapping_mul(3)));",
            "f.body[8..][..8].copy_from_slice(&i64::to_ne_bytes(-(n as i64)));",
            "cells[..4].copy_from_slice(&f32::to_ne_bytes(1.5));",
            "heap[i as usize * 4..][..4].copy_from_slice(&u32::to_ne_bytes((n + i) as u32));",
            "u32::from_ne_bytes(heap[8..][..4].try_into().unwrap()),",
        ],
    );
}

/// What R69 does not translate keeps a refusal of its own.
#[test]
fn a_number_in_bytes_without_a_rule_is_refused() {
    let program = |body: &str| {
        format!(
            "#include <stdio.h>\n#include <string.h>\nstruct pair {{ int a, b; }};\n{body}\nint main(void) {{\n    unsigned char b[8] = {{1, 2, 3, 4, 5, 6, 7, 8}};\n    printf(\"%d\\n\", f(b));\n    return 0;\n}}\n"
        )
    };
    let cases = [
        (
            "int *view(unsigned char *b) { return (int *)b; }\nint f(unsigned char *b) { return *view(b); }",
            "unsupported: a pointer to bytes cast to a pointer to a number that is kept rather than read or stored at once: no rule yet, at line 4",
        ),
        (
            "int f(unsigned char *b) { int *q = (int *)b; b = b + 4; return *q + b[0]; }",
            "unsupported: a pointer to bytes cast to a pointer to a number that is kept rather than read or stored at once: no rule yet, at line 4",
        ),
        (
            "int f(unsigned char *b) { const int *r = &((int *)b)[1]; b[4] = 9; return *r; }",
            "unsupported: a number in bytes used as a place, such as its address: no rule yet, at line 4",
        ),
        (
            "int f(unsigned char *b) { int at[1] = {0}; int *q = (int *)(b + at[0]); at[0] = 4; return *q; }",
            "unsupported: a pointer to bytes cast to a pointer to a number that is kept rather than read or stored at once: no rule yet, at line 4",
        ),
        (
            "int f(unsigned char *b) { *(int *)b += 1; return b[0]; }",
            "unsupported: a number in bytes changed other than by `=`: no rule yet, at line 4",
        ),
        (
            "int f(unsigned char *b) { return ((int *)(b + 4))[-1]; }",
            "unsupported: a number in bytes before the pointer to the bytes: no rule yet, at line 4",
        ),
        (
            "int f(unsigned char *b) { signed char s[4] = {0, 1, 0, 0}; s[0] = b[0]; return *(int *)s; }",
            "unsupported: a number read or stored through a pointer to `signed char`: no rule yet, at line 4",
        ),
        (
            "int f(unsigned char *b) { int n = b[0]; unsigned char *c = (unsigned char *)&n; return c[0]; }",
            "unsupported: the bytes of a number read through a cast pointer: no rule yet, at line 4",
        ),
        (
            "int f(unsigned char *b) { long w[1] = {b[0]}; return *(int *)w; }",
            "unsupported: a cast between pointers to numbers of different sizes, at line 4",
        ),
        (
            "int f(unsigned char *b) { return ((struct pair *)b)->a; }",
            "unsupported: a cast to or from a pointer to a record, at line 4",
        ),
    ];
    for (at, (body, refusal)) in cases.into_iter().enumerate() {
        assert_eq!(
            translate(&format!("bytes_refused_{at}"), &program(body)).unwrap_err(),
            refusal,
            "{body}"
        );
    }
}
