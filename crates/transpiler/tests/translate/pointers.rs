use crate::common::{assert_lines, check, translate};

#[test]
fn pointers_that_do_not_own_are_borrows() {
    let rust = check(
        "borrows",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct account { int id; long balance; };
void deposit(struct account *a, long amount) { a->balance += amount; }
long balance(const struct account *a) { return a->balance; }
void swap(int *x, int *y) { int t = *x; *x = *y; *y = t; }
int sum(const int *values, int count) {
    int total = 0;
    for (int i = 0; i < count; i++) total += values[i];
    return total;
}
void fill(int *values, int count, int with) {
    for (int i = 0; i < count; i++) values[i] = with + i;
}
struct account *find(struct account *all, int count, int id) {
    for (int i = 0; i < count; i++)
        if (all[i].id == id) return &all[i];
    return NULL;
}
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct account accounts[3] = {{1, 10}, {2, 20}, {3, 30}};
    int values[5];
    fill(values, 5, n);
    int a = 1, b = 2;
    swap(&a, &b);
    struct account *found = find(accounts, 3, n);
    if (found != NULL) deposit(found, 5);
    if (!found) printf("none\n");
    deposit(&accounts[0], sum(values + 1, 4));
    int *last = &values[4];
    *last += 1;
    printf("%ld %ld %d %d %d\n", balance(&accounts[0]), balance(&accounts[1]), a, b, *last);
    return 0;
}
"#,
        &[&["2"], &["9"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn deposit(a: &mut Account, amount: i64) {",
            "fn balance(a: &Account) -> i64 {",
            "fn sum(values: &[i32], count: i32) -> i32 {",
            "fn fill(values: &mut [i32], count: i32, with: i32) {",
            "fn find(all: &mut [Account], count: i32, id: i32) -> Option<&mut Account> {",
            "swap(&mut a, &mut b);",
            "deposit(&mut accounts[0], sum(&values[1..], 4) as i64);",
            "if let Some(found) = found.as_deref_mut() {",
            "deposit(found, 5);",
        ],
    );
}

#[test]
fn a_tested_pointer_is_opened_once() {
    let rust = check(
        "guards",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct item { int key; int value; };
struct item *lookup(struct item *items, int count, int key) {
    for (int i = 0; i < count; i++)
        if (items[i].key == key) return &items[i];
    return NULL;
}
int bump(struct item *items, int count, int key) {
    struct item *found = lookup(items, count, key);
    if (found == NULL) return -1;
    found->value += 1;
    return found->value;
}
const int *larger(const int *a, const int *b) { return *a > *b ? a : b; }
int main(int argc, char *argv[]) {
    int key = atoi(argv[1]);
    struct item items[2] = {{1, 10}, {2, 20}};
    int other = 15;
    printf("%d %d %d\n", bump(items, 2, key), bump(items, 2, 1), *larger(&items[0].value, &other));
    return 0;
}
"#,
        &[&["1"], &["2"], &["3"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let Some(found) = found.as_deref_mut() else {",
            "found.value += 1;",
            "fn larger<'a>(a: &'a i32, b: &'a i32) -> &'a i32 {",
        ],
    );
}

/// Reading a pointer that was never assigned is undefined in C; the
/// translation must still compile, and stops where C would read through it.
#[test]
fn a_pointer_that_is_never_assigned_is_none() {
    let rust = check(
        "unset",
        r#"
#include <stdio.h>
#include <stdlib.h>
void show(const int *number) { printf("%d\n", *number); }
void unset(void) {
    int *number;
    show(number);
}
void set(void) {
    int value = 5;
    int *number = &value;
    show(number);
}
int main(int argc, char *argv[]) {
    if (atoi(argv[1])) {
        set();
    } else {
        unset();
    }
    return 0;
}
"#,
        &[&["1"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn show(number: Option<&i32>) {",
            "let number: Option<&i32> = None;",
            "show(Some(number));",
        ],
    );
}

#[test]
fn a_result_that_may_be_stored_through_is_kept_to_read() {
    let rust = check(
        "result-reborrow",
        r#"
#include <stdio.h>
struct entry { int id; int value; };
struct entry *find(struct entry *entries, int count, int id) {
    for (int i = 0; i < count; i++) {
        if (entries[i].id == id) return &entries[i];
    }
    return NULL;
}
int main(void) {
    struct entry entries[3] = {{1, 10}, {2, 20}, {3, 30}};
    struct entry *found = find(entries, 3, 2);
    if (found) found->value += 1;
    const struct entry *missing = find(entries, 3, 9);
    printf("%d %d\n", missing == NULL, entries[1].value);
    const struct entry *last = find(entries, 3, 3);
    printf("%d\n", last ? last->value : -1);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &["let missing: Option<&Entry> = find(&mut entries, 3, 9).map(|found| &*found);"],
    );
}

#[test]
fn a_borrow_passed_on_by_name_is_reborrowed() {
    let rust = check(
        "reborrow",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
struct counter { int n; };
void show(const struct counter *c) { printf("%d\n", c->n); }
void bump(struct counter *c) { c->n++; }
void twice(struct counter *c, char *name) {
    show(c);
    bump(c);
    show(c);
    bump(c);
    strcpy(name, "two");
    printf("%s %d\n", name, (int)strlen(name));
    c->n += 10;
    show(c);
}
int main(int argc, char *argv[]) {
    struct counter c = {atoi(argv[1])};
    char name[8] = "";
    twice(&c, name);
    show(&c);
    return 0;
}
"#,
        &[&["3"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "show(c);",
            "bump(c);",
            "copy_string(name, b\"two\\0\");",
            "println!(\"{} {}\", text_of(name), until_nul(name).len() as i32);",
            "twice(&mut c, &mut name);",
            "show(&c);",
        ],
    );
}

/// Rust reaches the element through the record, which must be writable.
#[test]
fn a_store_to_an_element_behind_a_record_s_pointer_needs_the_record_writable() {
    let rust = check(
        "element_behind_field",
        r#"#include <stdio.h>
#include <stdlib.h>
typedef struct { int n; } item;
typedef struct { item *items; int count; } shelf;
static void bump(item *one, int by) { one->n += by; }
static void bump_all(shelf *b, int by) {
    for (int i = 0; i < b->count; i++) bump(&b->items[i], by);
}
int main(int argc, char *argv[]) {
    shelf b;
    b.count = 3;
    b.items = malloc(sizeof(item) * 3);
    if (!b.items) return 1;
    for (int i = 0; i < 3; i++) b.items[i].n = i;
    bump_all(&b, argc);
    printf("%d %d %d\n", b.items[0].n, b.items[1].n, b.items[2].n);
    free(b.items);
    return 0;
}
"#,
        &[&[], &["a", "b"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn bump_all(b: &mut Shelf, by: i32) {",
            "bump(&mut b.items[i as usize], by);",
        ],
    );
}

#[test]
fn two_elements_of_an_array_one_before_the_other_are_split() {
    let rust = check(
        "elements_apart",
        r#"#include <stdio.h>
typedef struct { int n; int data[2]; } item;
static void copy(const item *from, item *to) {
    to->n = from->n + 1;
    to->data[1] = from->data[0] * 2;
}
static void swap(item *a, item *b) {
    item t = *a;
    *a = *b;
    *b = t;
}
static void spread(item *all, int count) {
    for (int i = 1; i < count; i++) copy(&all[0], &all[i]);
}
int main(int argc, char *argv[]) {
    item items[5];
    for (int i = 0; i < 5; i++) {
        items[i].n = i * argc;
        items[i].data[0] = i + argc;
        items[i].data[1] = 0;
    }
    spread(items, 3);
    for (int i = 4; i < 5; i += 2) copy(&items[i], &items[3]);
    swap(&items[1], &items[4]);
    for (int i = 0; i < 5; i++) printf("%d %d %d\n", items[i].n, items[i].data[0], items[i].data[1]);
    return 0;
}
"#,
        &[&[], &["a", "b"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let (lower, upper) = all.split_at_mut(i as usize);",
            "copy(&lower[0], &mut upper[0]);",
            "let (lower, upper) = items.split_at_mut(i as usize);",
            "copy(&upper[0], &mut lower[3]);",
            "let (lower, upper) = items.split_at_mut(4);",
            "swap(&mut lower[1], &mut upper[0]);",
        ],
    );
    let refused = |name: &str, body: &str| {
        let c = format!(
            "typedef struct {{ int n; }} item;\nstatic void copy(const item *from, item *to) {{ to->n = from->n; }}\nstatic void add(const int *from, int *to, int n) {{ for (int i = 0; i < n; i++) to[i] += from[i]; }}\nint main(int argc, char *argv[]) {{\n    item items[4] = {{{{1}}, {{2}}, {{3}}, {{4}}}};\n    int v[4] = {{1, 2, 3, 4}};\n{body}\n    return items[argc].n + v[3];\n}}\n"
        );
        crate::common::translate(name, &c).unwrap_err()
    };
    for (name, body) in [
        // The counter starts at the other element.
        (
            "apart_from_same",
            "    for (int i = 0; i < 4; i++) copy(&items[0], &items[i]);",
        ),
        ("apart_same", "    copy(&items[2], &items[2]);"),
        // The constant is not below where the counter starts.
        (
            "apart_above",
            "    for (int i = 0; i < 2; i++) copy(&items[3], &items[i]);",
        ),
        (
            "apart_down",
            "    for (int i = 3; i > 0; i--) copy(&items[0], &items[i]);",
        ),
        (
            "apart_variable",
            "    int at = argc; copy(&items[0], &items[at]);",
        ),
        (
            "apart_moved",
            "    for (int i = 1; i < 4; i++) { copy(&items[0], &items[i]); if (argc > 5) i--; }",
        ),
        // The condition moves the counter down to the other element.
        (
            "apart_condition",
            "    for (int i = 1; --i >= 0 && i < 3; i += 2) copy(&items[0], &items[i]);",
        ),
        // The index is the counter cut to a byte.
        (
            "apart_cut",
            "    for (int i = 256; i < 259; i++) copy(&items[0], &items[(unsigned char)i]);",
        ),
        // An unsigned counter wraps to the other element.
        (
            "apart_unsigned",
            "    for (unsigned i = 1; i < 2; i += 4294967295U) { copy(&items[0], &items[i]); if (i == 0) break; }",
        ),
        (
            "apart_narrow",
            "    for (unsigned char i = 1; i < 4; i++) copy(&items[0], &items[i]);",
        ),
        // The rest of the array from an element reaches the other one.
        ("apart_slices", "    add(&v[0], &v[2], 2);"),
    ] {
        assert_eq!(
            refused(name, body),
            "unsupported: two arguments that point into the same memory, one of them to store through, at line 7",
            "{name}"
        );
    }
}

/// A local pointer to a variable is a borrow that ends with its last use,
/// so the variable is free again after it, after the pointer gets the
/// address anew, and in a branch that the use is not in.
#[test]
fn a_variable_is_used_again_after_the_pointer_to_it() {
    let rust = check(
        "borrow_ends",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct gauge { int low; int high; };
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct gauge g = {1, 2};
    struct gauge *view = &g;
    view->low = n;
    g.high = view->low + 1;
    view = &g;
    view->high += 2;
    int levels[4] = {0};
    const int *top = &levels[3];
    int seen = levels[3] + *top;
    if (n > 3) {
        levels[3] = n;
    } else {
        seen += *top;
    }
    printf("%d %d %d %d\n", g.low, g.high, seen, levels[3]);
    return 0;
}
"#,
        &[&["2"], &["7"]],
        "",
    );
    assert_lines(&rust, &["view = &mut g;", "seen += *top;"]);
}

#[test]
fn an_element_and_the_one_a_constant_further_are_split() {
    // R54: `i` and `i + 1` are never the same element.
    let rust = check(
        "neighbours",
        r#"#include <stdio.h>
#include <stdlib.h>
static void trade(int *one, int *other) { int kept = *one; *one = *other; *other = kept; }
int main(int argc, char **argv) {
    int from = atoi(argv[1]);
    int row[7] = { 5, 3, 8, 1, 9, 2, 7 };
    for (int pass = 0; pass < 7; pass++) {
        for (int at = 0; at + 1 < 7; at++) {
            if (row[at] > row[at + 1]) trade(&row[at], &row[at + 1]);
        }
    }
    trade(&row[from + 2], &row[from]);
    long far = from;
    trade(&row[far - 1], &row[far]);
    for (int at = 0; at < 7; at++) printf("%d ", row[at]);
    printf("\n");
    return 0;
}
"#,
        &[&["1"], &["4"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let (lower, upper) = row.split_at_mut((at + 1) as usize);",
            "trade(&mut lower[at as usize], &mut upper[0]);",
            "let (lower, upper) = row.split_at_mut((from + 2) as usize);",
            "trade(&mut upper[0], &mut lower[from as usize]);",
            "let (lower, upper) = row.split_at_mut(far as usize);",
        ],
    );
    let refused = |name, index: &str, other: &str| {
        crate::common::translate(
            name,
            &format!(
                "static void trade(int *one, int *other) {{ int kept = *one; *one = *other; *other = kept; }}\nint main(int argc, char **argv) {{\n    int row[4] = {{ 1, 2, 3, 4 }};\n    unsigned u = 1; short s = 1; int i = 1, j = 1;\n    trade(&row[{index}], &row[{other}]);\n    return row[u] + row[s] + row[i] + row[j];\n}}\n"
            ),
        )
        .unwrap_err()
    };
    let reason = "unsupported: two arguments that point into the same memory, one of them to store through, at line 5";
    assert_eq!(refused("neighbours-unsigned", "u", "u + 1"), reason);
    assert_eq!(refused("neighbours-narrow", "s", "(short)(s + 1)"), reason);
    assert_eq!(refused("neighbours-other", "i", "j + 1"), reason);
    assert_eq!(refused("neighbours-none", "i", "i + 0"), reason);
    assert_eq!(refused("neighbours-effect", "i++", "i + 1"), reason);
}

/// R61: a local pointer that only ever holds the address of one place, and
/// that Rust would reject as a borrow beside the place, is that place.
#[test]
fn a_pointer_that_only_names_one_place_is_that_place() {
    let rust = check(
        "one_place",
        r#"
#include <stdio.h>
struct span { int from; int to; };
struct frame { struct span inner; int depth; };
void widen(struct span *s) { s->to += 10; }
int main(void) {
    struct span whole;
    struct span *w;
    w = &whole;
    whole.from = 2;
    w->to = 5;
    widen(w);
    struct frame f = {{1, 2}, 3};
    int *depth = &f.inner.to;
    f.inner.to = 8;
    *depth += f.depth;
    int cells[2][3] = {{0, 1, 2}, {3, 4, 5}};
    int *corner = &cells[1][2];
    int (*rows)[3] = cells;
    cells[1][2] = 9;
    *corner += rows[0][1];
    char word[4] = "abc";
    char *letters = word;
    word[0] = 'x';
    letters[2] = 'z';
    int total = 0;
    int *into = &total;
    for (int i = 0; i < 3; i++) {
        *into += i;
        total *= 2;
    }
    printf("%d %d %d %d %s %d\n", w->from, whole.to, *depth, rows[1][2], word, *into);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "whole.to = 5;",
            "widen(&mut whole);",
            "f.inner.to += f.depth;",
            "cells[1][2] += cells[0][1];",
            "word[2] = b'z';",
            "total += i;",
        ],
    );
}

/// R61: a local pointer to a pointer that only ever holds the address of
/// one pointer variable is that variable, also after the variable changes.
#[test]
fn a_pointer_to_a_pointer_that_only_names_one_pointer_is_that_pointer() {
    let rust = check(
        "pointer_to_one_pointer",
        r#"
#include <stdio.h>
struct pair { int left; int right; };
static int lo = 2, hi = 40;
static int both(const struct pair *at) {
    const struct pair **via = &at;
    return (*via)->left + via[0]->right;
}
static int after(int *from) {
    int **hold;
    hold = &from;
    return **hold + from[1];
}
int main(void) {
    int row[4] = {10, 20, 30, 40};
    int *walk = row, **step = &walk;
    printf("%d\n", **step);
    walk += 2;
    printf("%d\n", **step);
    *step = row + 1;
    printf("%d %d\n", *walk, (*step)[1]);
    int *bound, **ref;
    bound = &lo;
    ref = &bound;
    **ref += 5;
    bound = &hi;
    if (*ref) printf("%d %d\n", **ref, lo);
    struct pair p = {7, 8};
    printf("%d %d\n", both(&p), after(row + 2));
    int count = 1, *slot, **name;
    slot = &count;
    name = &slot;
    count += 5;
    **name += count;
    printf("%d\n", count);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "walk += 2;",
            "walk = 1;",
            "println!(\"{} {}\", row[walk], row[walk + 1]);",
            "count += count;",
        ],
    );
    assert!(!rust.contains("step") && !rust.contains("hold") && !rust.contains("via"));
}

/// R61 leaves a pointer to a pointer that is more than a second name for one
/// pointer variable as it is, refused as a pointer to a pointer.
#[test]
fn a_pointer_to_a_pointer_that_is_more_than_a_name_is_refused() {
    let head = "#include <stdio.h>\nstatic int peek(int **pp) { return **pp; }\nstatic int **same(int **pp) { return pp; }\n";
    for (name, body) in [
        ("twice", "int *q = &x, **pp = &p; pp = &q; return **pp;"),
        (
            "branch",
            "int **pp; if (x) pp = &p; else pp = &p; return **pp;",
        ),
        ("moved", "int **pp = &p; pp++; return pp[-1][0];"),
        ("index", "int **pp = &p; return pp[1] == 0;"),
        ("compared", "int **pp = &p; return pp == &p;"),
        ("handed", "int **pp = &p; return peek(pp);"),
        ("returned", "int **pp = &p; return **same(pp);"),
        ("address", "int **pp = &p; int ***r = &pp; return ***r;"),
        (
            "read_first",
            "int **pp = 0; if (pp) return 1; pp = &p; return **pp;",
        ),
        (
            "hidden",
            "int **pp = &p; { int *p = 0; return **pp + (p != 0); }",
        ),
        ("cast", "char **pp = (char **)&p; return **pp;"),
        ("triple", "int **q = &p; int ***pp = &q; return ***pp;"),
        (
            "loop",
            "int *q = &x, **pp; for (int i = 0; i < 2; i++) pp = i ? &q : &p; return **pp;",
        ),
        ("void", "void *v = p; void **pp = &v; return *(int *)*pp;"),
    ] {
        let c = format!("{head}int main(void) {{ int x = 1; int *p = &x; {body} }}\n");
        let refusal = translate(name, &c).unwrap_err();
        assert!(
            refusal.contains("a pointer to a pointer") || refusal.contains("behind a pointer"),
            "{name}: {refusal}"
        );
    }
    // A place that is no pointer variable is left as well, and refused for
    // what it is.
    for (name, c) in [
        (
            "field",
            "struct box { int *v; };\nint main(void) { int x = 2; struct box b; int **pp; b.v = &x; pp = &b.v; **pp = 3; return x; }\n",
        ),
        (
            "arrow",
            "struct box { int *v; };\nstatic int get(struct box *q) { int **pp = &q->v; return **pp; }\nint main(void) { int x = 2; struct box b = {&x}; return get(&b); }\n",
        ),
        (
            "element",
            "int main(void) { int x = 1; int *a[1]; int **pp; a[0] = &x; pp = &a[0]; return **pp; }\n",
        ),
        (
            "global",
            "int g = 4; int *gp = &g;\nint main(void) { int **pp = &gp; **pp = 8; return g; }\n",
        ),
    ] {
        assert!(translate(name, c).is_err(), "{name}");
    }
}

/// R61: an index before the element that a replaced pointer names is an
/// element of the array.
#[test]
fn an_offset_from_a_replaced_pointer_into_an_array_is_an_element_of_the_array() {
    let rust = check(
        "alias-offsets",
        r#"
#include <stdio.h>
int main(int argc, char **argv) {
    int steps[5] = {10, 20, 30, 40, 50};
    int *mid = &steps[2];
    int back = -argc;
    unsigned char one = 1;
    steps[1] = 21;
    printf("%d %d %d %d\n", mid[back], *(mid + back), mid[-1], *(mid - 1));
    steps[0] = 3;
    printf("%d %d\n", mid[-2], mid[one]);
    return 0;
}
"#,
        &[&[], &["more"]],
        "",
    );
    assert_lines(&rust, &["steps[(2 + back) as usize],", "steps[1],"]);
}

/// A local pointer read out of one field of a record stays a borrow of
/// that field while another field of the record is stored to: Rust tells
/// the fields apart.
#[test]
fn a_pointer_to_a_part_of_a_record_beside_another_field() {
    let rust = check(
        "part_beside",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct link { int load; struct link *rest; };
int main(void) {
    struct link *top = malloc(sizeof *top);
    top->rest = malloc(sizeof *top);
    top->rest->load = 6;
    top->rest->rest = NULL;
    struct link *second = top->rest;
    top->load = 2;
    second->load += top->load;
    printf("%d %d\n", second->load, top->load);
    free(top->rest);
    free(top);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(&rust, &["top.load = 2;"]);
}
