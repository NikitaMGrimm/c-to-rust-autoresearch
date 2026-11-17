use crate::common::{assert_lines, check, translate};

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

#[test]
fn realloc_resizes_the_array_its_pointer_owns() {
    let rust = check(
        "grow",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct stack { long *slots; int top; int room; };
static unsigned *marks;
static void put(struct stack *s, long value) {
    if (s->top == s->room) {
        s->room += 3;
        s->slots = (long *)realloc(s->slots, s->room * sizeof(long));
    }
    s->slots[s->top++] = value;
}
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    struct stack s;
    s.top = 0;
    s.room = 1;
    s.slots = malloc(sizeof *s.slots);
    for (int i = 0; i < n; i++) put(&s, (long)i * 7);
    float *w = malloc(2 * sizeof(float));
    w[0] = 0.5f;
    w[1] = 1.5f;
    w = realloc(w, sizeof(float) * (n + 2));
    for (int i = 2; i < n + 2; i++) w[i] = w[i - 1] + w[i - 2];
    char *word = malloc(2);
    word[0] = 'q';
    word[1] = 0;
    word = realloc(word, n + 2);
    for (int i = 1; i <= n; i++) word[i] = 'a' + i;
    word[n + 1] = 0;
    marks = malloc(sizeof(unsigned));
    marks[0] = 9;
    marks = realloc(marks, (n + 1) * sizeof *marks);
    for (int i = 1; i <= n; i++) marks[i] = marks[i - 1] + 2;
    int *seen = NULL;
    int count = 0;
    for (int i = 0; i < n; i += 2) {
        seen = realloc(seen, (count + 1) * sizeof(int));
        seen[count++] = i;
    }
    for (int i = 0; i < s.top; i++) printf("%ld ", s.slots[i]);
    printf("\n%.1f %s %u\n", w[n + 1], word, marks[n]);
    for (int i = 0; i < count; i++) printf("%d ", seen[i]);
    printf("\n");
    free(s.slots);
    free(w);
    free(word);
    free(marks);
    free(seen);
    return 0;
}
"#,
        &[&["1"], &["5"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "slots: Option<Vec<i64>>,",
            "let grown_len = s.room as usize;",
            "s.slots.get_or_insert_with(Vec::new).resize(grown_len, 0);",
            "w.get_or_insert_with(Vec::new).resize(grown_len, 0.0);",
            "word.get_or_insert_with(Vec::new).resize(grown_len, 0);",
            "globals.marks.get_or_insert_with(Vec::new).resize(grown_len, 0);",
            "let grown_len = (count + 1) as usize;",
            "seen.get_or_insert_with(Vec::new).resize(grown_len, 0);",
        ],
    );
}

#[test]
fn realloc_to_a_size_of_zero_frees_the_array_as_the_library_does() {
    let rust = check(
        "shrink",
        r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    int *a = malloc(2 * sizeof *a);
    a[0] = n;
    a[1] = 8;
    a = realloc(a, a[0] * sizeof *a);
    puts(a ? "kept" : "freed");
    if (a) printf("%d\n", a[0]);
    a = realloc(a, n * sizeof *a);
    puts(a ? "kept" : "freed");
    free(a);
    a = NULL;
    int none = 0;
    a = realloc(a, none * sizeof *a);
    puts(a ? "kept" : "freed");
    free(a);
    int *b = malloc(sizeof *b);
    b[0] = 5;
    b = realloc(b, 4 * sizeof *b);
    b[3] = n;
    printf("%d %d\n", b[0], b[3]);
    free(b);
    return 0;
}
"#,
        &[&["0"], &["3"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let grown_len = a.as_deref().unwrap()[0] as usize;",
            "if grown_len == 0 && a.is_some() {",
            "a = None;",
            "a.get_or_insert_with(Vec::new).resize(grown_len, 0);",
            "b.resize(4, 0);",
        ],
    );
}

#[test]
fn realloc_that_is_no_resize_in_place_is_refused() {
    let refused = |name, body: &str| {
        let c = format!(
            "#include <stdlib.h>\nint main(void) {{\n    int *a = malloc(2 * sizeof(int));\n{body}    free(a);\n    return 0;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    let elsewhere = "unsupported: `realloc` whose result is not stored back in the pointer it grows: no rule yet, at line 4";
    assert_eq!(
        refused("bytes", "    a = realloc(a, 40);\n"),
        "unsupported: a size of `realloc` with no rule, at line 4"
    );
    assert_eq!(
        refused("none", "    a = realloc(a, 0);\n"),
        "unsupported: a constant size of `realloc` that keeps no array, at line 4"
    );
    assert_eq!(
        refused(
            "wide",
            "    unsigned long n = 3;\n    a = realloc(a, n * sizeof(int));\n"
        ),
        "unsupported: a count of `realloc` as wide as a size, whose product C may wrap, at line 5"
    );
    assert_eq!(
        refused(
            "cut",
            "    int n = 3;\n    a = realloc(a, (unsigned char)(n * sizeof(int)));\n"
        ),
        "unsupported: a size of `realloc` with no rule, at line 5"
    );
    assert_eq!(
        refused(
            "cut-constant",
            "    a = realloc(a, (unsigned char)(65ULL * sizeof(int)));\n"
        ),
        "unsupported: a constant size of `realloc` that keeps no array, at line 4"
    );
    assert_eq!(
        refused(
            "narrow-product",
            "    unsigned n = 3;\n    a = realloc(a, n * (unsigned)sizeof(int));\n"
        ),
        "unsupported: a size of `realloc` with no rule, at line 5"
    );
    assert_eq!(
        refused(
            "careful",
            "    int *b = realloc(a, 3 * sizeof(int));\n    if (!b) return 1;\n    a = b;\n"
        ),
        elsewhere
    );
    assert_eq!(
        refused(
            "tested",
            "    if ((a = realloc(a, 3 * sizeof(int))) == NULL) return 1;\n"
        ),
        elsewhere
    );
    assert_eq!(
        translate(
            "tested_null",
            "#include <stdlib.h>\nint main(void) {\n    int *a = NULL;\n    if (!(a = realloc(a, sizeof(int)))) return 1;\n    free(a);\n    return 0;\n}\n"
        )
        .unwrap_err(),
        elsewhere
    );
    assert_eq!(
        refused("dropped", "    realloc(a, 3 * sizeof(int));\n"),
        elsewhere
    );
    assert_eq!(
        translate(
            "parameter",
            "#include <stdlib.h>\nint *more(int *a) {\n    a = realloc(a, 4 * sizeof(int));\n    return a;\n}\nint main(void) {\n    int *a = malloc(sizeof(int));\n    a = more(a);\n    free(a);\n    return 0;\n}\n"
        )
        .unwrap_err(),
        "unsupported: `realloc` of a parameter's array, at line 3"
    );
}

/// R66: after `gone = list;` both names hold one pointer until either is
/// stored to, and Rust has moved it: the reads of `list` name `gone`.
#[test]
fn a_read_of_an_owner_follows_its_memory_to_the_local_that_got_it() {
    let chain = |walk: &str| {
        format!(
            "#include <stdio.h>\n#include <stdlib.h>\nstruct link {{ int weight; struct link *rest; }};\nstatic struct link *grow(int count) {{\n    struct link *list = NULL;\n    for (int i = 1; i <= count; i++) {{\n        struct link *fresh = malloc(sizeof(struct link));\n        fresh->weight = i * 3;\n        fresh->rest = list;\n        list = fresh;\n    }}\n    return list;\n}}\nint main(void) {{\n    struct link *list = grow(4);\n    int total = 0;\n{walk}    printf(\"%d\\n\", total);\n    return 0;\n}}\n"
        )
    };
    let rust = check(
        "follow-walk",
        &chain(
            "    while (list != NULL) {\n        struct link *gone;\n        gone = list;\n        total += list->weight;\n        list = list->rest;\n        free(gone);\n    }\n",
        ),
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "total += gone.as_deref().unwrap().weight;",
            "list = gone.as_deref_mut().unwrap().rest.take();",
        ],
    );
    // A parameter, read in a condition and in both branches.
    let rust = check(
        "follow-param",
        "#include <stdio.h>\n#include <stdlib.h>\nstruct link { int weight; struct link *rest; };\nstatic void report(struct link *list) {\n    struct link *kept = list;\n    if (list->rest) printf(\"%d then %d\\n\", list->weight, list->rest->weight);\n    else printf(\"%d alone\\n\", list->weight);\n    free(kept->rest);\n    free(kept);\n}\nint main(int argc, char **argv) {\n    struct link *one = malloc(sizeof(struct link));\n    one->weight = 7;\n    one->rest = NULL;\n    if (argc > 1) {\n        one->rest = malloc(sizeof(struct link));\n        one->rest->weight = 9;\n        one->rest->rest = NULL;\n    }\n    report(one);\n    return 0;\n}\n",
        &[&[], &["more"]],
        "",
    );
    assert_lines(&rust, &["if kept.rest.is_some() {"]);
    // What stays refused: a store to one of the names in a branch, a
    // `case` that the store does not reach, a jump, and a local that hides
    // the new name.
    let reason = |line: u32| {
        format!(
            "unsupported: a pointer that is used after its memory went to another owner, at line {line}"
        )
    };
    for (name, walk, line) in [
        (
            "follow-branch",
            "    struct link *gone = list;\n    if (total == 0) { list = list->rest; }\n    total += list->weight;\n    free(gone);\n",
            18,
        ),
        (
            "follow-case",
            "    struct link *gone = NULL;\n    switch (total) {\n    case 1: gone = list;\n    case 0: total += list->weight; gone = list;\n    }\n    free(gone);\n",
            20,
        ),
        (
            "follow-jump",
            "    struct link *gone = NULL;\n    if (total) goto count;\n    gone = list;\ncount:\n    total += list->weight;\n    free(gone);\n",
            21,
        ),
        (
            "follow-hidden",
            "    struct link *gone = list;\n    { int gone = 2; total += list->weight + gone; }\n    free(gone);\n",
            18,
        ),
    ] {
        assert_eq!(
            translate(name, &chain(walk)).unwrap_err(),
            reason(line),
            "{name}"
        );
    }
}

/// What a field gives up as one value of a conditional is taken out of
/// its record, which the record's binding must allow.
#[test]
fn a_field_that_a_conditional_gives_up_is_taken_out_of_a_writable_record() {
    let rust = check(
        "conditional-move",
        "#include <stdio.h>\n#include <stdlib.h>\nstruct link { int weight; struct link *rest; };\nint main(int argc, char **argv) {\n    struct link *list = NULL;\n    for (int i = 1; i <= 3; i++) {\n        struct link *fresh = malloc(sizeof(struct link));\n        fresh->weight = i;\n        fresh->rest = list;\n        list = fresh;\n    }\n    int total = 0;\n    while (list) {\n        struct link *gone = list;\n        total += gone->weight;\n        list = argc > 1 || gone->weight > 1 ? gone->rest : NULL;\n        free(gone);\n    }\n    printf(\"%d\\n\", total);\n    return 0;\n}\n",
        &[&[], &["all"]],
        "",
    );
    assert_lines(&rust, &["let mut gone: Option<Box<Link>> = list;"]);
}

#[test]
fn a_field_that_is_moved_out_and_read_again_in_one_statement_is_refused() {
    // The move leaves `None` in the field, which the other read would see
    // where C still sees the pointer.
    let program = |call: &str| {
        format!(
            "#include <stdio.h>\n#include <stdlib.h>\nstruct box {{ int *p; }};\nvoid consume(int *p, int present) {{ printf(\"%d %d\\n\", *p, present); free(p); }}\nvoid pop(struct box *s) {{\n    {call}\n    s->p = NULL;\n}}\nint main(void) {{ struct box b; b.p = malloc(sizeof *b.p); *b.p = 7; pop(&b); return 0; }}\n"
        )
    };
    assert_eq!(
        translate("again", &program("consume(s->p, s->p != NULL);")).unwrap_err(),
        "unsupported: a move out of a field that is read again before it is stored to: no rule yet, at line 6"
    );
    assert_eq!(
        translate(
            "later",
            &program("int *q = s->p;\n    printf(\"%d\\n\", s->p != NULL);\n    free(q);")
        )
        .unwrap_err(),
        "unsupported: a move out of a field that is read again before it is stored to: no rule yet, at line 6"
    );
    // A function of the program may read the field, and so may another
    // way to write the record.
    for (name, later) in [
        ("called", "pop(s);"),
        (
            "other-element",
            "s[1].p = NULL;\n    printf(\"%d\\n\", s[0].p != NULL);",
        ),
        ("written", "printf(\"%d\\n\", (&*s)->p != NULL);"),
    ] {
        assert_eq!(
            translate(
                name,
                &program(&format!("int *q = s->p;\n    {later}\n    free(q);"))
            )
            .unwrap_err(),
            "unsupported: a move out of a field that is read again before it is stored to: no rule yet, at line 6",
            "{name}"
        );
    }
    assert!(translate("once", &program("consume(s->p, 1);")).is_ok());
    assert!(
        translate(
            "stored",
            &program("int *q = s->p;\n    s->p = NULL;\n    free(q);")
        )
        .is_ok()
    );
}

/// `a->rest->rest` reads the field `rest` twice, but the first read only
/// leads to the second: a move out of the second leaves the first.
#[test]
fn a_field_read_on_the_way_to_the_one_that_is_moved_out_is_not_read_again() {
    let program = |end: &str| {
        format!(
            "#include <stdio.h>\n#include <stdlib.h>\nstruct link {{ int weight; struct link *rest; }};\nstruct link *fresh(int weight, struct link *rest) {{\n    struct link *made = malloc(sizeof(struct link));\n    made->weight = weight;\n    made->rest = rest;\n    return made;\n}}\nint peek(struct link *at, int more) {{ return at->weight + more; }}\nint main(void) {{\n    struct link *list = fresh(1, fresh(2, fresh(3, NULL)));\n    printf(\"%d\\n\", list->rest->rest->weight);\n    {end}\n    free(list->rest);\n    free(list);\n    return 0;\n}}\n"
        )
    };
    let rust = check("path", &program("free(list->rest->rest);"), &[&[]], "");
    assert_lines(
        &rust,
        &["drop(list.rest.as_deref_mut().unwrap().rest.take());"],
    );
    // The value that the function returns at its end is a statement too.
    assert!(
        translate(
            "returned",
            &program("return peek(list->rest, list->rest->rest->weight);")
        )
        .is_ok()
    );
    assert!(
        translate(
            "returned-move",
            "#include <stdio.h>\n#include <stdlib.h>\nstruct link { int weight; struct link *rest; };\nint consume(struct link *at, int weight) { printf(\"%d\\n\", weight); free(at); return 0; }\nint main(void) {\n    struct link *list = calloc(1, sizeof *list);\n    list->rest = calloc(1, sizeof *list->rest);\n    list->rest->rest = calloc(1, sizeof *list->rest->rest);\n    return consume(list->rest, list->rest->rest->weight);\n}\n"
        )
        .unwrap_err()
        .starts_with(
            "unsupported: a move out of a field that is read again before it is stored to: no rule yet"
        )
    );
    // The same field beside the path, or the place read again later, is
    // a second read.
    for (name, end) in [
        (
            "beside",
            "printf(\"%d\\n\", peek(list->rest, list->rest->rest != NULL));\n    free(list->rest->rest);\n    printf(\"%d\\n\", list->rest->rest != NULL);",
        ),
        (
            "later",
            "struct link *gone = list->rest->rest;\n    printf(\"%d\\n\", list->rest->rest != NULL);\n    free(gone);",
        ),
    ] {
        assert!(
            translate(name, &program(end)).unwrap_err().starts_with(
                "unsupported: a move out of a field that is read again before it is stored to: no rule yet"
            ),
            "{name}"
        );
    }
}

/// An owner that a conditional hands on is translated where nothing names
/// it again before a store to it.
#[test]
fn a_conditional_hands_on_one_of_two_owners() {
    let rust = check(
        "conditional_owner",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct jar { int beans; };
static int empty(struct jar *j) { int n = j->beans; free(j); return n; }
static struct jar *fill(int beans) {
    struct jar *j = malloc(sizeof *j);
    j->beans = beans;
    return j;
}
int main(int argc, char **argv) {
    struct jar *left = fill(3), *right = fill(8);
    int n = empty(argc > 1 ? left : right);
    left = fill(n);
    right = fill(n + 1);
    printf("%d %d %d\n", n, left->beans, right->beans);
    free(left);
    free(right);
    return 0;
}
"#,
        &[&[], &["x"]],
        "",
    );
    assert_lines(
        &rust,
        &["let n = empty(if args.len() as i32 > 1 { left } else { right });"],
    );
}

#[test]
fn a_list_freed_by_hand_from_its_end_is_dropped() {
    let rust = check(
        "freed_by_hand",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct knot { int weight; struct knot *below; };
static struct knot *tie(int weight, struct knot *below) {
    struct knot *k = malloc(sizeof *k);
    if (!k) exit(1);
    k->weight = weight;
    k->below = below;
    return k;
}
static int doubled(int v) { return 2 * v; }
int main(void) {
    struct knot *top = tie(1, tie(2, tie(3, NULL)));
    printf("%d %d\n", top->below->weight, top->below->below->weight);
    free(top->below->below);
    free(top->below);
    free(top);
    printf("%d\n", doubled(4));
    return 0;
}
"#,
        &[&[]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "drop(top.below.as_deref_mut().unwrap().below.take());",
            "drop(top.below.take());",
            "drop(top);",
        ],
    );
}
