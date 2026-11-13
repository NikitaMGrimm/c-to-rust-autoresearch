use crate::common::{assert_lines, check, translate};

#[test]
fn a_pointer_that_walks_an_array_is_an_index() {
    let rust = check(
        "cursors",
        r#"
#include <stdio.h>
#include <stddef.h>
struct entry { int id; int value; };
int total(char *s) {
    int n = 0;
    while (*s) { n += *s; s++; }
    return n;
}
struct entry *find(struct entry *entries, int size, int id) {
    struct entry *ptr = entries;
    struct entry *end = entries + size;
    while (ptr < end) {
        if (ptr->id == id) return ptr;
        ptr++;
    }
    return NULL;
}
int backward(int *array, size_t size, int start) {
    int sum = 0;
    int *ptr;
    int *first;
    ptr = array + size;
    first = array + start;
    while (ptr > first) { ptr--; sum += *ptr; }
    return sum;
}
int count(const char *in, char c) {
    int n = 0;
    for (const char *s = in; *s; s++) {
        if (*s == c) n++;
    }
    return n;
}
const char *skip(const char *s) {
    while (*s == ' ') s++;
    return s;
}
int main(void) {
    char text[8] = "  abc a";
    struct entry entries[3] = {{1, 10}, {2, 20}, {3, 30}};
    int numbers[6] = {1, 2, 3, 4, 5, 6};
    struct entry *found = find(entries, 3, 2);
    if (found) found->value += 1;
    char *p = text;
    while (*p && *p != 'c') p++;
    long distance = p - text;
    char *q = p + 1;
    int step = -2;
    int *n = numbers + 4;
    n += step;
    printf("%d %d %d %d %d\n", total(text), entries[1].value, backward(numbers, 6, 2),
           count(text, 'a'), backward(numbers, 6, 6));
    printf("%s|%ld %ld %ld %d %d %d %d\n", skip(text), distance, q - p, p - q, p == text,
           *n, n[-1], n[step + 3]);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            // A parameter that is moved stays the slice.
            "fn total(s: &[u8]) -> i32 {",
            "let mut s_at: usize = 0;",
            "while s[s_at] != 0 {",
            "s_at += 1;",
            // Two cursors over one array are compared as indices.
            "let mut ptr: usize = 0;",
            "let end: usize = size as usize;",
            "while ptr < end {",
            "if entries[ptr].id == id {",
            "return Some(&mut entries[ptr..]);",
            "let mut ptr: usize = size as usize;",
            "let first: usize = start as usize;",
            "ptr -= 1;",
            "sum += array[ptr];",
            // The rest of the array from a cursor.
            "&s[s_at..]",
            "let distance: i64 = p as i64;",
            "let q: usize = p + 1;",
            "n = n.wrapping_add_signed(step as isize);",
        ],
    );
}

#[test]
fn a_cursor_may_step_through_values_and_tests() {
    let rust = check(
        "cursor-values",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int store[5] = {1, 2, 3, 4, 5};
static int used = 5;
void copy(char *dst, const char *src) {
    while (*src) { *dst++ = *src++; }
    *dst = '\0';
}
int length(const char *s) {
    int n = 0;
    while (s && *s) { n++; s++; }
    return n;
}
int main(void) {
    char from[8] = "a,b,c", to[8];
    copy(to, from);
    char *p = to;
    int commas = 0;
    while (*p != '\0') {
        if (*p++ == ',') commas++;
    }
    int *end = &store[used];
    int *it = end;
    int last = 0;
    used--;
    for (int i = 0; i < 3 && it > store; i++) { it--; last += *it; }
    int *heap = malloc(4 * sizeof(int));
    int *h = heap;
    for (int i = 0; i < 4; i++) { *h = i * i; h++; }
    int squares = 0;
    for (h = heap; h < heap + 4; h++) squares += *h;
    free(heap);
    int *first = store;
    int *second;
    second = first + 2;
    first = second;
    first--;
    printf("%s %d %ld %d %d %d %d %d %d\n", to, commas, (long)(p - to), last, squares,
           length(to), length(NULL), *first, *second);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            // NULL reaches the slice, not the index.
            "fn length(s: Option<&[u8]>) -> i32 {",
            "let end: usize = globals.used as usize;",
            "let mut it: usize = end;",
            "while i < 3 && it > 0 {",
            "last += STORE[it];",
            "while h < 4 {",
            "squares += heap[h];",
            "let second: usize = first + 2;",
            "first = second;",
        ],
    );
}

#[test]
fn a_cursor_handed_on_may_be_the_end_of_its_array() {
    // C may hand on the address one past the last element; Rust has no
    // element there, but an empty rest of the array.
    let rust = check(
        "cursor-end",
        r#"
#include <stdio.h>
int *after(int *p, int n) {
    while (n-- > 0) p++;
    return p;
}
int show(const int *p, int inside) {
    return inside ? *p : -1;
}
int main(int argc, char **argv) {
    int a[3] = {4, 5, 6};
    printf("%d\n", show(after(a, argc - 1), argc < 4));
    return 0;
}
"#,
        &[&[], &["x"], &["x", "y", "z"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn after(p: &[i32], mut n: i32) -> &[i32] {",
            "&p[p_at..]",
            "if inside != 0 { p[0] } else { -1 }",
        ],
    );
}

/// A pointer without one array that it walks keeps its refusal.
#[test]
fn a_pointer_that_walks_no_single_array_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "two-arrays",
            "int main(void) {\n    int a[2] = {1, 2}, b[2] = {3, 4};\n    int *p = a;\n    int *q = b;\n    p++;\n    return p < q;\n}\n"
        ),
        "unsupported: a comparison of two addresses, at line 6"
    );
    assert_eq!(
        refused(
            "two-bases",
            "int main(int argc, char **argv) {\n    int a[2] = {1, 2}, b[2] = {3, 4};\n    int *p = a;\n    if (argc > 1) p = b;\n    p++;\n    return *p;\n}\n"
        ),
        "unsupported: `++` or `--` on a pointer: no rule yet, at line 5"
    );
    // `base` is another array by the time `p` is read.
    assert_eq!(
        refused(
            "moved-base",
            "int main(void) {\n    int a[3] = {1, 2, 3}, b[3] = {4, 5, 6};\n    int *base = a;\n    int *p = base;\n    p++;\n    base = b;\n    return *p + *base;\n}\n"
        ),
        "unsupported: `++` or `--` on a pointer: no rule yet, at line 5"
    );
    // Inside the block, `a` names other memory.
    assert_eq!(
        refused(
            "hidden-base",
            "int main(void) {\n    int a[3] = {1, 2, 3};\n    int *p = a;\n    p++;\n    {\n        int a[3] = {7, 8, 9};\n        return *p + a[0];\n    }\n}\n"
        ),
        "unsupported: `++` or `--` on a pointer: no rule yet, at line 4"
    );
}
