use crate::common::{assert_lines, check};

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
