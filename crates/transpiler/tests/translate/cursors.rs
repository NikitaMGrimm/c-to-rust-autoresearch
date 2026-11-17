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
            "let end = size as usize;",
            "while ptr < end {",
            "if entries[ptr].id == id {",
            "return Some(&mut entries[ptr..]);",
            "let mut ptr = size as usize;",
            "let first = start as usize;",
            "ptr -= 1;",
            "sum += array[ptr];",
            // The rest of the array from a cursor.
            "&s[s_at..]",
            "let distance = p as i64;",
            "let q = p + 1;",
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
            "let end = globals.used as usize;",
            "let mut it = end;",
            "while i < 3 && it > 0 {",
            "last += STORE[it];",
            "while h < 4 {",
            "squares += heap[h];",
            "let second = first + 2;",
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

#[test]
fn a_cursor_that_steps_before_its_array_is_refused() {
    let refused = |name, body: &str| {
        let c = format!(
            "#include <stdio.h>\nint main(void) {{\n    int a[3] = {{4, 5, 6}};\n    int sum = 0;\n{body}    printf(\"%d\\n\", sum);\n    return 0;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    // The last `p--` leaves the pointer before `a`, and only that ends
    // the loop.
    assert_eq!(
        refused("down", "    for (int *p = a + 2; p >= a; p--) sum += *p;\n"),
        "unsupported: a cursor that steps before its array, at line 5"
    );
    // The second step back is from the first element.
    assert_eq!(
        refused(
            "twice",
            "    int *p = &a[1];\n    sum += *(p--);\n    sum += *(p--);\n"
        ),
        "unsupported: a cursor that steps before its array, at line 7"
    );
    assert_eq!(
        refused(
            "by_two",
            "    int *p = a;\n    p++;\n    if (sum == 0) p -= 2;\n    p += 2;\n    sum += *p;\n"
        ),
        "unsupported: a cursor that steps before its array, at line 7"
    );
}

#[test]
fn a_cursor_that_steps_back_inside_its_array_stays() {
    let rust = check(
        "back",
        r#"
#include <stdio.h>
int main(void) {
    int a[3] = {4, 5, 6};
    int sum = 0;
    int *p = &a[2];
    sum += *(p--);
    sum += *(p--);
    sum += *p;
    for (int *q = a + 3; q > a;) {
        q--;
        sum += *q;
    }
    printf("%d\n", sum);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(&rust, &["let mut p: usize = 2;", "q -= 1;"]);
}

#[test]
fn a_parameter_that_is_the_end_of_another_parameters_array_is_an_index() {
    let rust = check(
        "bounded",
        r#"
#include <stdio.h>
struct cell { int age; int mark; };
static int rising(const int *at, const int *stop, int ceiling) {
    int run = 0;
    while (at != stop && *at <= ceiling) { run++; at++; }
    return run;
}
static long width(const int *stop, const int *first) { return (stop - first) * 10 + *first; }
static int youngest(const struct cell *c, const struct cell *past) {
    int least = c->age;
    for (; c < past; c++) if (c->age < least) least = c->age;
    return least;
}
static int closing(const unsigned char *from, const unsigned char *upto) {
    return from == upto ? -1 : *(upto - 1) + upto[-1];
}
int main(void) {
    int heights[7] = {2, 3, 5, 8, 1, 4, 6};
    struct cell cells[3] = {{40, 1}, {12, 2}, {33, 3}};
    unsigned char bytes[5] = {9, 8, 7, 6, 250};
    for (int limit = 1; limit < 9; limit += 3)
        printf("%d %d %d\n", rising(heights, heights + 7, limit),
               rising(&heights[2], &heights[6], limit), rising(heights + 4, heights + 4, limit));
    for (int n = 0; n <= 7; n += 7)
        printf("%ld %ld\n", width(heights + n, heights), width(heights + 5, heights + 2));
    printf("%d %d\n", youngest(cells, cells + 3), youngest(cells + 2, cells + 3));
    printf("%d %d\n", closing(bytes, bytes + 5), closing(bytes + 1, bytes + 1));
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn rising(at: &[i32], stop: usize, ceiling: i32) -> i32 {",
            "let mut at_at: usize = 0;",
            "while at_at != stop && at[at_at] <= ceiling {",
            "rising(&heights, 7, limit),",
            "rising(&heights[2..], 4, limit),",
            "rising(&heights[4..], 0, limit),",
            "fn width(stop: usize, first: &[i32]) -> i64 {",
            "println!(\"{} {}\", width(n as usize, &heights), width(3, &heights[2..]));",
            "fn youngest(c: &[Cell], past: usize) -> i32 {",
            "fn closing(from: &[u8], upto: usize) -> i32 {",
        ],
    );
}

#[test]
fn an_end_that_the_calls_do_not_prove_is_refused() {
    let refused = |name, c: &str| translate(name, c).unwrap_err();
    let walk = "static int walk(const int *p, const int *end) {\n    int s = 0;\n    while (p < end) s += *p++;\n    return s;\n}\n";
    let main = |body: &str| {
        format!(
            "#include <stdio.h>\n{walk}int main(void) {{\n    int a[4] = {{1, 2, 3, 4}};\n{body}    return 0;\n}}\n"
        )
    };
    let comparison = "unsupported: a comparison of two addresses, at line 4";
    // The arguments name the array in two ways.
    let other = main("    const int *q = a;\n    printf(\"%d\\n\", walk(q, a + 2));\n");
    assert_eq!(refused("other-base", &other), comparison);
    // A call through a pointer proves nothing.
    let pointer = main(
        "    int (*f)(const int *, const int *) = walk;\n    printf(\"%d\\n\", f(a, a + 2));\n",
    );
    assert_eq!(refused("through-pointer", &pointer), comparison);
    // The end may be before `p`, which Rust's index cannot say.
    let before = main("    printf(\"%d\\n\", walk(a + 3, a + 1));\n");
    assert_eq!(refused("before", &before), comparison);
    // The offset of a pointer that may point into the middle of an array.
    let middle = main(
        "    const int *q = a + 2;\n    for (int n = 0; n < 2; n++) printf(\"%d\\n\", walk(q, q + n));\n",
    );
    assert_eq!(refused("middle", &middle), comparison);
    // A call in the offset.
    let text = "#include <stdio.h>\n#include <string.h>\nstatic int walk(const char *p, const char *end) {\n    int s = 0;\n    while (p < end) s += *p++;\n    return s;\n}\nint main(void) {\n    const char *s = \"abc\";\n    printf(\"%d\\n\", walk(s, s + strlen(s)));\n    return 0;\n}\n";
    assert_eq!(
        refused("call-in-offset", text),
        "unsupported: a comparison of two addresses, at line 5"
    );
    // The function moves the end itself.
    let stored = "static int trim(const int *p, const int *end) {\n    while (p < end && end[-1] == 0) end--;\n    return (int)(end - p);\n}\nint main(void) {\n    int a[3] = {1, 0, 0};\n    return trim(a, a + 3);\n}\n";
    assert_eq!(
        refused("stored", stored),
        "unsupported: a comparison of two addresses, at line 2"
    );
    // The function calls itself with an end that is no offset of the array.
    let itself = "static int sum(const int *p, const int *end) {\n    return p == end ? 0 : *p + sum(p + 1, end);\n}\nint main(void) {\n    int a[3] = {1, 2, 3};\n    return sum(a, a + 3);\n}\n";
    assert_eq!(
        refused("itself", itself),
        "unsupported: a comparison of two addresses, at line 2"
    );
    // An end handed on needs `end - p` as an index, which may be below zero.
    let handed = format!(
        "#include <stdio.h>\n{walk}static int skip(const int *p, const int *end) {{\n    while (p < end && *p < 0) p++;\n    return walk(p, end);\n}}\nint main(void) {{\n    int a[4] = {{-1, 2, 3, 4}};\n    printf(\"%d\\n\", skip(a, a + 4));\n    return 0;\n}}\n"
    );
    assert_eq!(refused("handed-on", &handed), comparison);
    // NULL is no offset of an array.
    let null = main("    printf(\"%d\\n\", walk(NULL, NULL) + walk(a, a + 1));\n");
    assert_eq!(refused("null", &null), comparison);
}
