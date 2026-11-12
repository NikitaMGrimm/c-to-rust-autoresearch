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
