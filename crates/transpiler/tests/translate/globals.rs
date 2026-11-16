use crate::common::{assert_lines, check, translate};

#[test]
fn globals_that_change_are_passed_as_one_struct() {
    let rust = check(
        "globals",
        r#"
#include <stdio.h>
#include <stdlib.h>
typedef struct { int floors; double baths; } house_t;
static house_t the_house = {.floors = 2, .baths = 1.5};
static const int limit = 4;
static int primes[5] = {2, 3, 5, 7, 11};
static int calls;
int next(void) {
    static int at = 0;
    calls++;
    return primes[at++ % 5];
}
void add_floor(house_t *house) { house->floors++; }
void grow(void) { if (the_house.floors < limit) add_floor(&the_house); }
int floors(void) { return the_house.floors; }
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    for (int i = 0; i < n; i++) { grow(); the_house.baths += next(); }
    printf("%d %.1f %d\n", floors(), the_house.baths, calls);
    return 0;
}
"#,
        &[&["0"], &["7"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "const LIMIT: i32 = 4;",
            "static PRIMES: [i32; 5] = [2, 3, 5, 7, 11];",
            "struct Globals {",
            "fn next(globals: &mut Globals) -> i32 {",
            "fn floors(globals: &Globals) -> i32 {",
            "add_floor(&mut globals.the_house);",
            "let globals = &mut Globals {",
        ],
    );
}

#[test]
fn the_zero_of_a_record_that_never_changes_is_written_out() {
    // A `static` may call no function, so `Shelf::default()` would not
    // compile there.
    let rust = check(
        "fixed-records",
        r#"#include <stdio.h>
struct slot { int row; double share; };
struct shelf { int width; struct slot first; char label[3]; };
struct shelf empty;
struct shelf begun = { 9 };
struct slot slots[2];
struct shelf moved;
int main(void) {
    moved.first.row = 5;
    printf("%d %d %.1f %d\n", empty.width, empty.first.row, empty.first.share, empty.label[1]);
    printf("%d %d %d %.1f %d\n", begun.width, begun.first.row, slots[1].row, slots[0].share, moved.first.row);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "static EMPTY: Shelf = Shelf {",
            "first: Slot { row: 0, share: 0.0 },",
            "static SLOTS: [Slot; 2] = [Slot { row: 0, share: 0.0 }; 2];",
        ],
    );
    assert!(rust.contains("moved: Shelf::default()"));
}

/// A pointer into a global goes to a function that takes no globals, or
/// as a shared borrow to one that only reads them.
#[test]
fn a_pointer_to_a_global_is_handed_on_beside_the_globals_it_fits() {
    let rust = check(
        "global_pointers",
        r#"
#include <stdio.h>
#include <stdlib.h>
int heap[4], left[4], used;
void put(int *into, int value) { into[0] = value; into[1] = value * 2; }
int peek(const int *from) { return from[1] + used; }
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    used = n;
    put(heap, n);
    put(left, n + 1);
    printf("%d %d %d\n", peek(heap), peek(left), heap[0] + left[0]);
    return 0;
}
"#,
        &[&["3"], &["8"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "put(&mut globals.heap, n);",
            "peek(globals, &globals.heap),",
        ],
    );
}

#[test]
fn a_global_pointer_that_never_changes_is_the_place_it_names() {
    let rust = check(
        "fixed-pointers",
        r#"#include <stdio.h>
struct gauge { int low; int high; };
struct gauge dial = { 2, 11 };
int steps[4] = { 5, 6, 7, 8 };
int level = 3;
int *lp = &level;
int *upper = &dial.high;
struct gauge *dp = &dial;
int *row = steps;
int *last = &steps[3];
void raise(int by) { static int *keep = &level; *keep += by; }
int main(int argc, char *argv[]) {
    raise(argc * 4);
    int sum = 0;
    for (int i = 0; i < 4; i++) sum += row[i];
    printf("%d %d %d %d %d\n", *lp, *upper, dp->low, sum, *last);
    return 0;
}
"#,
        &[&[], &["x"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "static DIAL: Gauge = Gauge { low: 2, high: 11 };",
            "globals.level += by;",
            "sum += STEPS[i as usize];",
            "println!(\"{} {} {} {} {}\", globals.level, DIAL.high, DIAL.low, sum, STEPS[3]);",
        ],
    );
}

#[test]
fn an_index_through_a_fixed_pointer_into_an_array_is_an_index_of_the_array() {
    let rust = check(
        "fixed-middle",
        r#"#include <stdio.h>
#include <stdlib.h>
int marks[5] = { 4, 9, 16, 25, 36 };
enum { LATE = 3 };
int *mid = &marks[1 + 1];
struct view { int id; int *from; };
struct view pane = { 7, &marks[LATE] };
int main(int argc, char *argv[]) {
    int n = argc > 1 ? atoi(argv[1]) : 0;
    long far = n;
    mid[n] += 1;
    printf("%d %d %d %d %d\n", mid[n], mid[-1], pane.from[n], pane.from[-3], mid[far]);
    return 0;
}
"#,
        &[&[], &["-2"], &["1"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "globals.marks[(2 + n) as usize] += 1;",
            "globals.marks[(3 + n) as usize],",
            "globals.marks[(2 + far) as usize],",
        ],
    );
}

#[test]
fn a_record_literal_that_a_global_pointer_holds_is_a_global() {
    let rust = check(
        "literal-globals",
        r#"#include <stdio.h>
struct inner { int u; int v; };
struct outer { struct inner in; int w; };
struct inner *fixed = &(struct inner){ 7, 1 };
struct outer *kept = &(struct outer){ .w = 4, .in = { .v = 9 } };
int main(void) {
    kept->w *= 3;
    printf("%d %d %d %d %d\n", fixed->u, fixed->v, kept->in.u, kept->in.v, kept->w);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "static FIXED: Inner = Inner { u: 7, v: 1 };",
            "kept: Outer {",
            "globals.kept.w *= 3;",
        ],
    );
}

#[test]
fn a_pointer_field_that_never_changes_is_the_place_it_names() {
    let rust = check(
        "fixed-fields",
        r#"#include <stdio.h>
int fuel = 40;
int marks[3] = { 2, 4, 6 };
struct tank { int size; int *level; int *row; };
struct tank small = { .level = &fuel, .size = 5, .row = marks };
struct tank big = { 9, &marks[1], &marks[0] };
struct tank *main_tank = &big;
static int peek(int *at) { return *at + 1; }
static int use(void) {
    static struct tank spare = { 3, &fuel, marks };
    spare.size += 2;
    return spare.size + *spare.level;
}
int main(int argc, char *argv[]) {
    *small.level -= argc;
    printf("%d %d %d\n", small.size, *small.level, peek(small.level));
    printf("%d %d %d\n", *big.level, main_tank->row[2], main_tank->size);
    int first = use();
    printf("%d %d\n", first, use());
    return 0;
}
"#,
        &[&[], &["x"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "static BIG: Tank = Tank { size: 9 };",
            "globals.fuel -= args.len() as i32;",
            "println!(\"{} {} {}\", MARKS[1], MARKS[2], BIG.size);",
            "globals.spare.size + globals.fuel",
        ],
    );
}

#[test]
fn a_pointer_field_that_changes_or_is_reached_otherwise_is_refused() {
    let owned = "it owns memory and also points into a variable";
    let prelude = "#include <stdio.h>\n#include <string.h>\nint k = 1, j = 2;\nstruct hold { int *p; int n; };\n";
    let cases = [
        (
            "stored",
            "struct hold h = { &k, 1 };\nint main(void) { h.p = &j; return *h.p; }\n",
        ),
        (
            "null",
            "struct hold h = { &k, 1 }, e = { 0, 2 };\nint main(void) { return *h.p + e.n; }\n",
        ),
        (
            "arrow",
            "struct hold h = { &k, 1 };\nstatic int f(struct hold *q) { return *q->p; }\nint main(void) { return f(&h); }\n",
        ),
        (
            "local",
            "struct hold h = { &k, 1 };\nint main(void) { struct hold l = { &j, 2 }; return *h.p + *l.p; }\n",
        ),
        (
            "sized",
            "struct hold h = { &k, 1 };\nint main(void) { return *h.p + (int)sizeof(struct hold); }\n",
        ),
        (
            "copied",
            "struct hold h = { &k, 1 }, g = { &j, 2 };\nint main(void) { g = h; return *g.p; }\n",
        ),
        (
            "memcpy",
            "struct hold h = { &k, 1 }, g = { &j, 2 };\nint main(void) { memcpy(&g, &h, 16); return *g.p; }\n",
        ),
        (
            "address",
            "struct hold h = { &k, 1 };\nint main(void) { int **at = &h.p; return **at; }\n",
        ),
        (
            "itself",
            "struct hold h = { &h.n, 1 };\nint main(void) { return *h.p; }\n",
        ),
    ];
    for (name, body) in cases {
        let error = translate(name, &format!("{prelude}{body}")).unwrap_err();
        assert!(error.contains(owned), "{name}: {error}");
    }
}
