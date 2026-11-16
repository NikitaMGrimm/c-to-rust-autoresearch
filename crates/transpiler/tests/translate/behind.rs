use crate::common::{assert_lines, check, translate};

#[test]
fn a_parameter_that_points_to_a_list_head_borrows_the_head() {
    let rust = check(
        "behind_list",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct cell { int v; struct cell *next; };
struct deck { struct cell *cards; int n; };
void push(struct cell **top, int v) {
    struct cell *c = malloc(sizeof *c);
    c->v = v;
    c->next = *top;
    *top = c;
}
void push_two(struct cell **top, int v) { push(top, v); push(top, v * 2); }
int size(struct cell **top) {
    int k = 0;
    for (struct cell *p = *top; p; p = p->next) k++;
    return k;
}
int peek(struct cell **top) { return *top == NULL ? -1 : (*top)->v; }
void grow(struct cell **top) { if (*top) (*top)->v += 50; }
void skip(struct cell **top) { if (!*top) return; *top = (*top)->next; }
void pop(struct cell **top) {
    if (*top == NULL) return;
    struct cell *old = *top;
    *top = old->next;
    free(old);
}
void under(struct deck *d, int v) { push(&d->cards->next, v); }
int main(int argc, char *argv[]) {
    struct cell *stack = NULL;
    struct deck d = { NULL, 0 };
    printf("%d %d\n", size(&stack), peek(&stack));
    for (int i = 0; i < argc + 2; i++) push(&stack, i + 1);
    push_two(&stack, 7);
    grow(&stack);
    printf("%d %d\n", size(&stack), peek(&stack));
    skip(&stack);
    pop(&stack);
    printf("%d %d\n", size(&stack), peek(&stack));
    while (stack) pop(&stack);
    push(&d.cards, 1);
    under(&d, 2);
    printf("%d %d %d\n", size(&stack), size(&d.cards), d.cards->next->v);
    while (d.cards) pop(&d.cards);
    return 0;
}
"#,
        &[&[], &["a", "b"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn push(top: &mut Option<Box<Cell>>, v: i32) {",
            "c.next = (*top).take();",
            "*top = Some(c);",
            "push(top, v);",
            "fn size(top: &Option<Box<Cell>>) -> i32 {",
            "fn peek(top: &Option<Box<Cell>>) -> i32 {",
            "(*top).as_deref_mut().unwrap().v += 50;",
            "*top = (*top).as_deref_mut().unwrap().next.take();",
            "let mut old: Option<Box<Cell>> = (*top).take();",
            "push(&mut d.cards.as_deref_mut().unwrap().next, v);",
            "push(&mut stack, i + 1);",
            "size(&stack),",
            "push(&mut d.cards, 1);",
        ],
    );
}

#[test]
fn a_parameter_that_gets_an_array_borrows_the_owner_of_it() {
    let rust = check(
        "behind_array",
        r#"
#include <stdio.h>
#include <stdlib.h>
void cubes(long **into, int n) {
    *into = malloc(n * sizeof(long));
    for (int i = 0; i < n; i++) (*into)[i] = (long)i * i * i;
}
void letters(char **into, int n) {
    *into = malloc(n + 1);
    for (int i = 0; i < n; i++) (*into)[i] = 'k' + i;
    (*into)[n] = '\0';
}
int main(int argc, char *argv[]) {
    long *c = NULL;
    char *w = NULL;
    int n = argc + 3;
    cubes(&c, n);
    letters(&w, n);
    long sum = 0;
    for (int i = 0; i < n; i++) sum += c[i];
    printf("%ld %s\n", sum, w);
    free(c);
    free(w);
    return 0;
}
"#,
        &[&[], &["x"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn cubes(into: &mut Option<Vec<i64>>, n: i32) {",
            "fn letters(into: &mut Option<Vec<u8>>, n: i32) {",
            "cubes(&mut c, n);",
        ],
    );
}

#[test]
fn a_pointer_to_a_pointer_without_a_rule_is_refused() {
    let refused = |name: &str, c: &str| translate(name, c).unwrap_err();
    let list = "#include <stdlib.h>\nstruct n { int v; struct n *next; };\n";
    let push = "void push(struct n **h, int v) { struct n *x = malloc(sizeof *x); x->v = v; x->next = NULL; x->next = *h; *h = x; }\n";
    assert_eq!(
        refused(
            "behind_global",
            &format!(
                "{list}{push}struct n *all = NULL;\nint main(void) {{ push(&all, 3); return all->v; }}\n"
            )
        ),
        "unsupported: the address of a global's pointer for a pointer to a pointer: the callee would borrow the globals twice, at line 5"
    );
    // C still holds the pointer that Rust would take out of `*pp`.
    assert_eq!(
        refused(
            "behind_kept",
            &format!(
                "{list}{push}void copy(struct n **to, struct n **from) {{ *to = *from; }}\nint main(void) {{ struct n *a = NULL, *b = NULL; push(&a, 5); copy(&b, &a); int r = b->v + (a ? a->v : 0); free(a); return r; }}\n"
            )
        ),
        "unsupported: a move out of the place behind a pointer that something may read before it is stored to: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "behind_early",
            &format!(
                "{list}{push}void drop_if(struct n **h, int k) {{ struct n *t = *h; if (t->v != k) return; *h = t->next; free(t); }}\nint main(void) {{ struct n *l = NULL; push(&l, 1); drop_if(&l, 9); int r = l ? l->v : 0; drop_if(&l, 1); return r; }}\n"
            )
        ),
        "unsupported: a move out of the place behind a pointer that something may read before it is stored to: no rule yet, at line 4"
    );
    let moved = "unsupported: a move out of the place behind a pointer that something may read before it is stored to: no rule yet, at line 3";
    let take = "#include <stdio.h>\n#include <stdlib.h>\n";
    // Another operand of the moving statement reads the place after the move.
    assert_eq!(
        refused(
            "behind_same_test",
            &format!(
                "{take}void eat(long *q, int full) {{ printf(\"%ld %d\\n\", *q, full); free(q); }} void take(long **slot) {{ eat(*slot, *slot != NULL); *slot = NULL; }}\nint main(void) {{ long *x = NULL; x = malloc(sizeof *x); *x = 11; take(&x); return 0; }}\n"
            )
        ),
        moved
    );
    assert_eq!(
        refused(
            "behind_same_read",
            &format!(
                "{take}void eat(long *q, long w) {{ printf(\"%ld %ld\\n\", *q, w); free(q); }} void take(long **slot) {{ eat(*slot, **slot + 1); *slot = NULL; }}\nint main(void) {{ long *x = NULL; x = malloc(sizeof *x); *x = 11; take(&x); return 0; }}\n"
            )
        ),
        moved
    );
    // The value stored back may read the place through a call.
    assert_eq!(
        refused(
            "behind_store_call",
            &format!(
                "{take}long *fresh(long **slot) {{ long *q = malloc(sizeof *q); *q = *slot ? 20 : 30; return q; }} void swap_in(long **slot) {{ long *old = *slot; *slot = fresh(slot); printf(\"%ld %ld\\n\", *old, **slot); free(old); }}\nint main(void) {{ long *x = NULL; x = malloc(sizeof *x); *x = 11; swap_in(&x); free(x); return 0; }}\n"
            )
        ),
        moved
    );
    // `first` borrows `h` while `h` is lent to be stored to.
    assert_eq!(
        refused(
            "behind_lent",
            &format!(
                "{list}{push}int main(void) {{ struct n *h = NULL; h = malloc(sizeof *h); h->v = 4; h->next = NULL; struct n *first = h; push(&h, 6); int r = first->v + h->v; free(h->next); free(h); return r; }}\n"
            )
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 4"
    );
    assert_eq!(
        refused(
            "behind_twice",
            &format!(
                "{list}void swap(struct n **a, struct n **b) {{ struct n *t = *a; *a = *b; *b = t; }}\nint main(void) {{ struct n *p = NULL; p = malloc(sizeof *p); p->v = 1; p->next = NULL; swap(&p, &p); int v = p ? p->v : 0; free(p); return v; }}\n"
            )
        ),
        "unsupported: two arguments of one call that name the same place, or a place and the pointer in it: Rust takes the pointer before it lends the place, at line 4"
    );
    assert_eq!(
        refused(
            "behind_local",
            &format!(
                "{list}{push}void last(struct n **h, int v) {{ struct n **at = h; while (*at) at = &(*at)->next; push(at, v); }}\nint main(void) {{ struct n *h = NULL; last(&h, 1); return h->v; }}\n"
            )
        ),
        "unsupported: a pointer to a pointer, at line 4"
    );
    assert_eq!(
        refused(
            "behind_cursor",
            "int skip(const char **s) { while (**s == ' ') (*s)++; return **s; }\nint main(void) { const char *t = \"  x\"; return skip(&t); }\n"
        ),
        "unsupported: the place behind the pointer `s`: a cursor behind a pointer: no rule yet, at line 1"
    );
    assert_eq!(
        refused(
            "behind_add",
            "void on(int **p, int n) { *p += n; }\nint main(void) { int a[3] = {1, 2, 3}; int *q = a; on(&q, 1); return *q; }\n"
        ),
        "unsupported: the place behind the pointer `p`: a cursor behind a pointer: no rule yet, at line 1"
    );
    assert_eq!(
        refused(
            "behind_borrow",
            "#include <stdio.h>\nvoid name(const char **out, int k) { *out = k ? \"on\" : \"off\"; }\nint main(void) { const char *r = \"?\"; name(&r, 1); puts(r); return 0; }\n"
        ),
        "unsupported: the place behind the pointer `out`: a borrow behind a pointer: no rule yet, at line 2"
    );
    assert_eq!(
        refused(
            "behind_compare",
            "#include <stdlib.h>\nint same(int **a, int **b) { return a == b; }\nint main(void) { int *p = malloc(sizeof *p); int *q = malloc(sizeof *q); int r = same(&p, &q); free(p); free(q); return r; }\n"
        ),
        "unsupported: a comparison of two addresses, at line 2"
    );
    assert_eq!(
        refused(
            "behind_function",
            "static int one(void) { return 1; }\nvoid set(int (**f)(void)) { *f = one; }\nint main(void) { int (*g)(void) = 0; set(&g); return g(); }\n"
        ),
        "unsupported: a pointer to a pointer to a function, at line 2"
    );
    assert_eq!(
        refused(
            "behind_offset",
            "#include <stdlib.h>\nint at(int **rows, int k) { return **(rows + k); }\nint main(void) { int *p = malloc(sizeof *p); *p = 4; int r = at(&p, 0); free(p); return r; }\n"
        ),
        "unsupported: a pointer to a pointer, at line 2"
    );
    assert_eq!(
        refused(
            "behind_result",
            &format!(
                "{list}struct n **first(struct n **h) {{ return h; }}\nint main(void) {{ struct n *h = malloc(sizeof *h); h->next = NULL; h->v = 1; int r = (*first(&h))->v; free(h); return r; }}\n"
            )
        ),
        "unsupported: a pointer to a pointer"
    );
    assert_eq!(
        refused(
            "behind_three",
            "#include <stdlib.h>\nvoid deep(int ***p) { **p = malloc(sizeof(int)); ***p = 2; }\nint main(void) { int *x = 0; int **y = &x; deep(&y); int r = *x; free(x); return r; }\n"
        ),
        "unsupported: a pointer to a pointer to a pointer, at line 2"
    );
}
