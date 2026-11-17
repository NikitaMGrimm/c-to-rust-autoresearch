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
            "let origin = PointT { x: 0, y: 0 };",
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

/// The length of an array names the variable that is in scope where the
/// array is declared.
#[test]
fn a_computed_length_is_the_variable_in_scope() {
    let rust = check(
        "length-scope",
        r#"
#include <stdio.h>
int n = 3;
int main(void) {
    { int n = 2; printf("%d\n", n); }
    int a[n];
    a[2] = 7;
    for (int n = 5; n < 6; n++) {
        int b[n];
        b[4] = a[2] + n;
        printf("%d\n", b[4]);
    }
    int c[n];
    c[2] = 1;
    printf("%d %d\n", a[2], c[2]);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut a: Vec<i32> = vec![0; N as usize];",
            "let mut b: Vec<i32> = vec![0; n as usize];",
            "let mut c: Vec<i32> = vec![0; N as usize];",
        ],
    );
}

#[test]
fn a_field_set_from_the_variable_of_its_name_is_named_once() {
    let rust = check(
        "field_shorthand",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct node { int id; int value; int weight; };
int main(int argc, char *argv[]) {
    int id = atoi(argv[1]);
    int value = id * 2;
    int other = id + 1;
    struct node n = {id, value, other};
    printf("%d %d %d\n", n.id, n.value, n.weight);
    return 0;
}
"#,
        &[&["3"]],
        "",
    );
    assert_lines(&rust, &["let n = Node { id, value, weight: other };"]);
}

#[test]
fn a_local_whose_value_fixes_its_type_has_no_annotation() {
    let rust = check(
        "inferred",
        r#"
#include <stdio.h>
#include <stdlib.h>
typedef struct { int x; int y; } point_t;
point_t at(int x) { point_t p = {x, x + 1}; return p; }
int twice(int n) { return 2 * n; }
int main(int argc, char *argv[]) {
    point_t p = at(atoi(argv[1]));
    int n = twice(p.y);
    int k = 5;
    long wide = (long)n * k;
    printf("%d %d %ld\n", p.x, n, wide);
    return 0;
}
"#,
        &[&["3"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let p = PointT { x, y: x + 1 };",
            "let p = at(parse_int(&args[1]) as i32);",
            "let n = twice(p.y);",
            "let k: i32 = 5;",
            "let wide = n as i64 * k as i64;",
        ],
    );
}

#[test]
fn a_record_without_a_tag_is_named_after_its_variable() {
    let rust = check(
        "untagged-vars",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct { int hits; long sum; } stats;
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct { int lo; int hi; } span = {n, n + 10}, wider, *at = &span;
    struct { char code; int weight; } marks[3] = {{'a', 1}, {'b', 2}, {'c', 3}};
    at->hi += 5;
    wider = span;
    for (int i = 0; i < 3; i++) {
        stats.hits++;
        stats.sum += marks[i].weight * n;
    }
    printf("%d %d %d %d %ld %c %zu\n", span.hi, wider.lo, wider.hi, stats.hits, stats.sum, marks[2].code,
           sizeof(marks) / sizeof(marks[0]));
    return 0;
}
"#,
        &[&["0"], &["4"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "struct Stats {",
            "struct Span {",
            "struct Marks {",
            "let mut span = Span { lo: n, hi: n + 10 };",
            "let at: &mut Span = &mut span;",
            "let marks: [Marks; 3] = [",
            "let wider = span;",
        ],
    );
}

#[test]
fn a_field_without_a_tag_is_named_after_the_field() {
    let rust = check(
        "untagged-fields",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct size { int w; };
struct panel { struct { int x; int y; } corner; struct { int w; int h; } size; enum { FLAT, RAISED } style; };
typedef struct { struct { unsigned short r, g, b; } tint; } brush;
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct panel p = {{n, 2}, {3, 4}, RAISED};
    struct size s = {9};
    brush b = {{1, 2, 3}};
    enum { LEFT, RIGHT } side = n > 2 ? RIGHT : LEFT;
    b.tint.g = (unsigned short)(b.tint.g + n);
    p.size.h *= 2;
    printf("%d %d %d %d %d %d\n", p.corner.x + p.corner.y, p.size.w * p.size.h, p.style, s.w, b.tint.g, side);
    return 0;
}
"#,
        &[&["1"], &["6"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "corner: Corner,",
            "size: PanelSize,",
            "struct Size {",
            "struct Tint {",
            "tint: Tint { r: 1, g: 2, b: 3 },",
            "style: u32,",
        ],
    );
}

#[test]
fn an_anonymous_member_is_a_field_that_holds_its_fields() {
    let rust = check(
        "untagged-anonymous",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct token { int kind; struct { int start; int end; }; union { float weight; unsigned bits; }; };
static int width(const struct token *t) { return t->end - t->start; }
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct token a = {1, {n, n + 4}};
    struct token b = {.kind = 2, .end = 9, .start = n};
    a.weight = 0.5f;
    b.end += a.kind;
    printf("%d %d %d %08x\n", width(&a), width(&b), b.kind, a.bits);
    return 0;
}
"#,
        &[&["2"], &["-3"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "anonymous: Anonymous,",
            "anonymous_: u32,",
            "t.anonymous.end - t.anonymous.start",
            "anonymous: Anonymous { start: n, end: 9 },",
            "a.anonymous_ = f32::to_bits(0.5);",
        ],
    );
}

#[test]
fn records_without_a_tag_in_two_scopes_are_two_types() {
    let rust = check(
        "untagged-scopes",
        r#"
#include <stdio.h>
static int first(int n) { struct { int v; } cell; cell.v = n; return cell.v + 1; }
static int second(int n) { struct { int v; } cell; cell.v = n * 3; return cell.v; }
int main(void) {
    printf("%d %d\n", first(2), second(5));
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut cell: Cell = Cell::default();",
            "let mut cell: Cell_ = Cell_::default();",
        ],
    );
}

#[test]
fn a_type_without_a_tag_that_has_no_rule_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "untagged-parameter",
            "#include <stddef.h>\nint count(struct { int n; } *p) {\n    return p != 0;\n}\nint main(void) {\n    return count(0);\n}\n"
        ),
        "unsupported: a type without a tag in a function's parameters or result, at line 2"
    );
    assert_eq!(
        refused(
            "untagged-result",
            "#include <stddef.h>\nstatic struct { int n; } *none(void) {\n    return 0;\n}\nint main(void) {\n    return none() != 0;\n}\n"
        ),
        "unsupported: a type without a tag in a function's parameters or result, at line 2"
    );
    assert_eq!(
        refused(
            "untagged-union",
            "struct slot { int tag; union { int n; double d; }; };\nint main(void) {\n    struct slot s;\n    s.n = 1;\n    return s.n - 1;\n}\n"
        ),
        "unsupported: a union whose members are not all numbers of one size: no rule yet"
    );
}

#[test]
fn a_record_gets_no_name_of_the_prelude() {
    // `struct Box` would hide the type that the allocation needs.
    let rust = check(
        "prelude-names",
        r#"#include <stdio.h>
#include <stdlib.h>
struct option { int chosen; };
int main(int argc, char **argv) {
    struct { int side; } box = { argc };
    struct { int length; } string = { 4 }, vec = { 5 };
    struct option some = { 6 };
    int *held = malloc(sizeof *held);
    *held = box.side + string.length + vec.length + some.chosen;
    const int *maybe = argc > 1 ? held : NULL;
    if (maybe) printf("%d %s\n", *maybe, argv[1]);
    else printf("%d\n", *held);
    free(held);
    return 0;
}
"#,
        &[&[], &["word"]],
        "",
    );
    assert_lines(
        &rust,
        &["struct Box_ {", "struct String_ {", "struct Option_ {"],
    );
}

#[test]
fn a_compound_literal_that_is_read_is_its_value() {
    let rust = check(
        "compound-literal",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct offset { int dx; int dy; };
struct span { struct offset from; struct offset to; int width; };
static struct offset home = (struct offset){ 3, -1 };
static struct offset scaled(struct offset v, int k) { return (struct offset){ v.dx * k, v.dy * k }; }
static int dot(struct offset a, struct offset b) { return a.dx * b.dx + a.dy * b.dy; }
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct offset v = (struct offset){ n, n + 1 };
    struct span s = { (struct offset){ 1, 2 }, { .dy = n }, 0 };
    int total = v.dy + s.from.dy + s.to.dy;
    v = (struct offset){ .dy = 7 };
    s = (struct span){ (struct offset){ n, 0 }, home };
    for (int i = 0; i < n; i++) {
        struct offset step = (struct offset){ i, 1 };
        step.dx += 2;
        total += dot(step, (struct offset){ 1, i });
    }
    double half = (double){ n } / 2;
    if ((struct offset){ n, 4 }.dy > n)
        total += ((struct offset){ 5, 6 }).dx;
    struct offset w = scaled(v, (int){ 2 });
    printf("%d %d %d %d %d %d %.1f %d %d\n", v.dx, v.dy, s.from.dx, s.to.dx, s.to.dy, s.width, half, total, w.dy);
    return 0;
}
"#,
        &[&["0"], &["3"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "static HOME: Offset = Offset { dx: 3, dy: -1 };",
            "fn scaled(v: Offset, k: i32) -> Offset {",
            "dx: v.dx * k,",
            "let mut v = Offset { dx: n, dy: n + 1 };",
            "from: Offset { dx: 1, dy: 2 },",
            "v = Offset { dx: 0, dy: 7 };",
            "let mut step = Offset { dx: i, dy: 1 };",
            "total += dot(step, Offset { dx: 1, dy: i });",
            "let half = n as f64 / 2.0;",
            "if (Offset { dx: n, dy: 4 }).dy > n {",
            "total += (Offset { dx: 5, dy: 6 }).dx;",
            "let w = scaled(v, 2);",
        ],
    );
}

#[test]
fn a_compound_literal_that_is_a_place_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    let program = |body: &str| {
        format!(
            "struct cell {{ int row; int col; }};\nint total(const int *v, int n) {{ return n > 0 ? v[0] : 0; }}\nint main(void) {{\n    {body}\n    return 0;\n}}\n"
        )
    };
    for (name, body, reason) in [
        (
            "literal-address",
            "struct cell *c = &(struct cell){ 1, 2 }; c->row = 4;",
            "a pointer to a compound literal: no rule yet",
        ),
        (
            "literal-store",
            "(struct cell){ 1, 2 }.col = 5;",
            "a compound literal as a place: no rule yet",
        ),
        (
            "literal-array",
            "return total((int[]){ 4, 5 }, 2);",
            "a compound literal that is an array: no rule yet",
        ),
        (
            "literal-string",
            "char *t = (char[]){ \"ab\" }; return t[0];",
            "a compound literal that is a string: no rule yet",
        ),
        (
            "literal-string-pointer",
            "const char *t = (const char *){ \"ab\" }; return t[0];",
            "a compound literal that is a string: no rule yet",
        ),
        (
            "literal-pointer",
            "int k = 2; int *q = (int *){ &k }; return *q;",
            "a compound literal that is a pointer: no rule yet",
        ),
    ] {
        assert_eq!(
            translate(name, &program(body)).unwrap_err(),
            format!("unsupported: {reason}, at line 4"),
            "{name}"
        );
    }
    assert_eq!(
        refused(
            "literal-address-global",
            "struct cell { int row; int col; };\nstatic const struct cell *first = &(struct cell){ 0, 1 };\nint main(void) {\n    return first->row;\n}\n"
        ),
        "unsupported: a pointer to a compound literal: no rule yet, at line 2"
    );
}

#[test]
fn a_pointer_field_that_nothing_stores_holds_null() {
    let rust = check(
        "unlinked",
        r#"
#include <stdio.h>
struct knot { int weight; struct knot *below; };
struct tray { int used; int *slots; };
int main(void) {
    struct tray t;
    t.used = 3;
    printf("%d\n", t.used);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &["below: Option<Box<Knot>>,", "slots: Option<Box<i32>>,"],
    );
}
