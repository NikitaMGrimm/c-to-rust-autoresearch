use crate::common::{assert_lines, check};

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
