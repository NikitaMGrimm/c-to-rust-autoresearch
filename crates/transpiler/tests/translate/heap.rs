use crate::common::{assert_lines, check};

#[test]
fn allocations_are_owned_and_freed_by_their_owner() {
    let rust = check(
        "heap",
        r#"
#include <stdio.h>
#include <stdlib.h>
typedef struct { int *data; size_t size; } buffer;
struct node { int value; struct node *next; };
buffer *make(size_t count, int first) {
    buffer *b = (buffer *)malloc(sizeof(buffer));
    if (!b) return NULL;
    b->data = (int *)calloc(count, sizeof(int));
    if (!b->data) { free(b); return NULL; }
    b->size = count;
    for (size_t i = 0; i < count; i++) b->data[i] = first + (int)i;
    return b;
}
long total(const buffer *b) {
    long sum = 0;
    for (size_t i = 0; i < b->size; i++) sum += b->data[i];
    return sum;
}
void release(buffer *b) {
    if (b) {
        if (b->data) free(b->data);
        free(b);
    }
}
struct node *push(struct node *head, int value) {
    struct node *n = malloc(sizeof(struct node));
    n->value = value;
    n->next = head;
    return n;
}
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    buffer *b = make(n, 3);
    if (!b) return 1;
    b->data[0] = 100;
    printf("%ld\n", total(b));
    release(b);
    int *one = malloc(sizeof(int));
    *one = n;
    double *many = malloc(n * sizeof(double));
    many[n - 1] = 2.5;
    printf("%d %.1f\n", *one, many[n - 1]);
    free(one);
    free(many);
    struct node *list = NULL;
    for (int i = 0; i < n; i++) list = push(list, i);
    int length = 0;
    for (const struct node *at = list; at; at = at->next) length += at->value;
    printf("%d\n", length);
    return 0;
}
"#,
        &[&["1"], &["6"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "data: Vec<i32>,",
            "next: Option<Box<Node>>,",
            "fn make(count: u64, first: i32) -> Box<Buffer> {",
            "let mut b: Box<Buffer> = Box::new(Buffer::default());",
            "b.data = vec![0; count as usize];",
            "fn total(b: &Buffer) -> i64 {",
            "fn release(mut b: Box<Buffer>) {",
            "fn push(head: Option<Box<Node>>, value: i32) -> Box<Node> {",
            "let mut many: Vec<f64> = vec![0.0; n as usize];",
            "drop(one);",
        ],
    );
}

#[test]
fn a_list_is_taken_apart_by_moving_its_nodes() {
    let rust = check(
        "list",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct node { int value; struct node *next; };
static struct node *stack = NULL;
static void push(int value) {
    struct node *n = malloc(sizeof(struct node));
    n->value = value;
    n->next = stack;
    stack = n;
}
static struct node *pop(struct node *head, int *out) {
    if (!head) return NULL;
    *out = head->value;
    struct node *rest = head->next;
    free(head);
    return rest;
}
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    for (int i = 1; i <= n; i++) push(i * i);
    int top = -1, sum = 0;
    struct node *list = stack;
    stack = NULL;
    list = pop(list, &top);
    while (list) {
        struct node *next = list->next;
        sum += list->value;
        free(list);
        list = next;
    }
    printf("%d %d\n", top, sum);
    return 0;
}
"#,
        &[&["0"], &["1"], &["5"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "stack: Option<Box<Node>>,",
            "fn push(globals: &mut Globals, value: i32) {",
            "n.next = globals.stack.take();",
            "globals.stack = Some(n);",
            "fn pop(head: Option<Box<Node>>, out: &mut i32) -> Option<Box<Node>> {",
            "let Some(mut head) = head else {",
            "let rest: Option<Box<Node>> = head.next.take();",
        ],
    );
}

#[test]
fn the_pointers_in_memory_from_calloc_are_null() {
    let rust = check(
        "calloc",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct holder { int *value; };
int main(void) {
    struct holder *h = calloc(1, sizeof *h);
    if (h->value) puts("set"); else puts("null");
    h->value = malloc(sizeof(int));
    *h->value = 3;
    if (h->value) printf("%d\n", *h->value);
    free(h->value);
    free(h);
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(&rust, &["value: Option<Box<i32>>,"]);
}

/// An owner that gave its memory away is free again once something is
/// stored to it, and a branch that did not move it may read it.
#[test]
fn an_owner_is_used_again_after_a_store_to_it() {
    let rust = check(
        "moved_then_stored",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct cell { int mark; struct cell *after; };
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct cell *list = NULL;
    for (int i = 0; i < n; i++) {
        struct cell *spare = malloc(sizeof *spare);
        spare->mark = i;
        spare->after = list;
        list = spare;
    }
    struct cell *last = malloc(sizeof *last);
    last->mark = 50;
    last->after = NULL;
    if (n > 2) {
        last->after = list;
        list = NULL;
    } else {
        printf("%d\n", last->mark);
    }
    while (list != NULL) {
        struct cell *next = list->after;
        printf("%d\n", list->mark);
        free(list);
        list = next;
    }
    while (last != NULL) {
        struct cell *next = last->after;
        printf("%d\n", last->mark);
        free(last);
        last = next;
    }
    return 0;
}
"#,
        &[&["1"], &["4"]],
        "",
    );
    assert_lines(&rust, &["list = Some(spare);", "list = None;"]);
}

#[test]
fn a_borrow_that_is_stored_through_walks_a_list() {
    // `p = p.as_deref_mut().unwrap().next.as_deref_mut()` does not compile:
    // the reborrow of `p` is in use where `p` is assigned.
    let rust = check(
        "walk-mut",
        r#"#include <stdio.h>
#include <stdlib.h>
struct bead { int weight; struct bead *after; };
static struct bead *string_of(int count) {
    struct bead *first = NULL;
    for (int i = count; i > 0; i--) {
        struct bead *fresh = malloc(sizeof(struct bead));
        fresh->weight = i;
        fresh->after = first;
        first = fresh;
    }
    return first;
}
static void heavier(struct bead *string, int by) {
    struct bead *at = string;
    while (at != NULL) {
        at->weight += by;
        at = at->after;
    }
}
static void weigh(struct bead *string, int skip, int weight) {
    struct bead *at = string;
    for (int i = 0; at != NULL && i < skip; i++) {
        at = at->after;
    }
    if (at != NULL) at->weight = weight;
}
int main(int argc, char **argv) {
    struct bead *string = string_of(atoi(argv[1]));
    heavier(string, 10);
    weigh(string, 1, 500);
    weigh(string, 40, 9);
    int sum = 0;
    for (const struct bead *at = string; at != NULL; at = at->after) sum += at->weight;
    printf("%d\n", sum);
    while (string != NULL) {
        struct bead *gone = string;
        string = gone->after;
        free(gone);
    }
    return 0;
}
"#,
        &[&["0"], &["1"], &["5"]],
        "",
    );
    assert_lines(&rust, &["at = at.take().unwrap().after.as_deref_mut();"]);
}
