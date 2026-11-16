use crate::common::translate;

/// What has no rule is refused with its reason and its line. There is no
/// other translation to fall back on.
#[test]
fn a_construct_without_a_rule_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "union",
            "union both { int i; double d; };\nint main(void) { return 0; }\n"
        ),
        "unsupported: a union whose members are not all numbers of one size: no rule yet"
    );
    assert_eq!(
        refused(
            "goto",
            "int main(int argc, char *argv[]) {\n    if (argc-- > 1) goto end;\n    {\nend:\n        argc++;\n    }\n    return 0;\n}\n"
        ),
        "unsupported: `goto end` into a block from outside it: no rule yet"
    );
    assert_eq!(
        refused(
            "addresses",
            "int main(void) {\n    int a, b;\n    int *p = &a, *q = &b;\n    return p < q;\n}\n"
        ),
        "unsupported: a comparison of two addresses, at line 4"
    );
}

/// What a rule would translate wrongly is refused too.
#[test]
fn a_program_that_a_rule_would_get_wrong_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    // `println!` has no value.
    assert_eq!(
        refused(
            "printed",
            "#include <stdio.h>\nint main(void) {\n    int n = printf(\"abc\\n\");\n    if (putchar('x') == 'x') n++;\n    return n;\n}\n"
        ),
        "unsupported: the value of `printf`: no rule yet, at line 3"
    );
    // Rust would step `i` twice.
    assert_eq!(
        refused(
            "twice",
            "int main(void) {\n    int a[3] = {1, 2, 3}, i = 0;\n    return a[i++]++;\n}\n"
        ),
        "unsupported: a place that calls or assigns and is updated, at line 3"
    );
    assert_eq!(
        refused(
            "tags",
            "struct s { int a; };\nint main(void) {\n    struct s { int b; };\n    struct s x = {7};\n    return x.b;\n}\n"
        ),
        "unsupported: the tag `s` with two meanings, at line 3"
    );
    assert_eq!(
        refused(
            "typedefs",
            "typedef int T;\nT g = 0;\nint main(void) {\n    typedef unsigned char T;\n    T n = 1;\n    return n + g;\n}\n"
        ),
        "unsupported: the typedef `T` with two meanings, at line 4"
    );
    assert_eq!(
        refused(
            "packed",
            "struct __attribute__((packed)) s { char a; int b; };\nint main(void) { return sizeof(struct s) == 5; }\n"
        ),
        "unsupported: a record that is packed or aligned"
    );
    assert_eq!(
        refused(
            "wide",
            "enum e { BIG = 1099511627776L };\nint main(void) { return sizeof(enum e) == 8; }\n"
        ),
        "unsupported: the type `enum e`, at line 2"
    );
    assert_eq!(
        refused(
            "width",
            "#include <stdio.h>\nint main(void) {\n    printf(\"|%4s|\\n\", \"\u{e9}\");\n    return 0;\n}\n"
        ),
        "unsupported: a `printf` width for text that is not ASCII, at line 3"
    );
    // The array ends where the function returns, and Rust lends nothing
    // that has ended.
    assert_eq!(
        refused(
            "returned",
            "#include <stdio.h>\nchar *name(void) {\n    char text[] = \"name\";\n    char *start = text;\n    return start;\n}\nint main(void) {\n    puts(name());\n    return 0;\n}\n"
        ),
        "unsupported: the result of `name`: it points to a variable that ends with the function"
    );
    // Rust lends memory once to a function that stores through it.
    assert_eq!(
        refused(
            "lent-twice",
            "void add(int *out, const int *in, int n) {\n    for (int i = 0; i < n; i++) out[i] += in[i];\n}\nint main(void) {\n    int a[4] = {1, 2, 3, 4};\n    add(a, a + 1, 3);\n    return a[0];\n}\n"
        ),
        "unsupported: two arguments that point into the same memory, one of them to store through, at line 6"
    );
}

/// A local pointer to a variable of the function is a borrow of it, so the
/// variable may not be stored to while the pointer is used later, nor read
/// while a pointer that is stored through is. A pointer that may name
/// another place than its one address stays such a borrow.
#[test]
fn a_variable_used_while_a_pointer_to_it_is_in_use_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "borrowed_twice",
            "int main(void) {\n    int low = 1, high = 2;\n    int *at = &low;\n    low = 3;\n    *at += 1;\n    at = &high;\n    return *at + low;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 5"
    );
    assert_eq!(
        refused(
            "borrowed_index",
            "int main(void) {\n    int slots[3] = {1, 2, 3}, which = 2;\n    int *slot = &slots[which];\n    slots[2] = 7;\n    return *slot - 7;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 5"
    );
    assert_eq!(
        refused(
            "borrowed_branch",
            "int main(int argc, char **argv) {\n    int level = 2;\n    int *raise;\n    if (argc > 0) raise = &level; else raise = &level;\n    level = 3;\n    return *raise - 3;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 6"
    );
    assert_eq!(
        refused(
            "borrowed_hidden",
            "int main(void) {\n    int mark = 2;\n    int *seen = &mark;\n    mark = 3;\n    {\n        int mark = 4;\n        return *seen + mark - 7;\n    }\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 7"
    );
    assert_eq!(
        refused(
            "borrowed_beside",
            "void put(int *to, int n) { *to = n; }\nint main(void) {\n    int depth = 1;\n    int *down = &depth;\n    depth = 2;\n    put(down, depth);\n    return depth - 2;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 6"
    );
    assert_eq!(
        refused(
            "borrowed_beside_again",
            "void put(int *to, int n) { *to = n; }\nint main(void) {\n    int depth = 1;\n    int *down = &depth;\n    depth = 2;\n    put(&*down, depth);\n    return depth - 2;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 6"
    );
    assert_eq!(
        refused(
            "borrowed_address",
            "int main(void) {\n    int depth = 1;\n    int *down = &depth;\n    int **again = &down;\n    depth = 3;\n    **again += 1;\n    return *down - 4;\n}\n"
        ),
        "unsupported: a variable that is used while a pointer to it is in use, at line 7"
    );
}

/// A function that uses the globals borrows `Globals` as a whole, so a
/// pointer into a global that changes fits beside it only as a shared
/// borrow beside `&Globals`.
#[test]
fn a_pointer_to_a_global_handed_to_a_function_that_uses_the_globals_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    let reason =
        "unsupported: a pointer to a global handed to a function that uses the globals, at line 6";
    // The callee changes a global and the pointer is stored through.
    assert_eq!(
        refused(
            "global_both_written",
            "int stock[3], moves;\nvoid shift(int *from) { from[0] = from[1]; moves++; }\nint main(void) {\n    stock[1] = 4;\n    moves = 0;\n    shift(stock);\n    return stock[0] + moves - 4;\n}\n"
        ),
        reason
    );
    // The callee changes a global and the pointer is only read.
    assert_eq!(
        refused(
            "global_read_beside_written",
            "int stock[3], moves;\nint shift(const int *from) { moves++; return from[1]; }\nint main(void) {\n    stock[1] = 4;\n    moves = 0;\n    int got = shift(stock);\n    return got + moves - 5;\n}\n"
        ),
        reason
    );
    // The callee only reads the globals and the pointer is stored through.
    assert_eq!(
        refused(
            "global_written_beside_read",
            "int stock[3], moves;\nvoid shift(int *from) { from[0] = from[1] + moves; }\nint main(void) {\n    stock[1] = 4;\n    moves = 1;\n    shift(stock);\n    return stock[0] - 5;\n}\n"
        ),
        reason
    );
    // Two pointers into one global, of which one is stored through, keep
    // their own reason.
    assert_eq!(
        refused(
            "global_twice",
            "int stock[3];\nvoid shift(int *to, const int *from) { to[0] = from[0]; }\nint main(void) {\n    stock[1] = 4;\n    shift(stock, stock + 1);\n    return stock[0] - 4;\n}\n"
        ),
        "unsupported: two arguments that point into the same memory, one of them to store through, at line 5"
    );
}

/// A local owner that hands its memory to another owner is moved in Rust,
/// so reading it again before a store to it is refused.
#[test]
fn an_owner_used_after_it_was_moved_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    let reason =
        "unsupported: a pointer that is used after its memory went to another owner, at line 8";
    assert_eq!(
        refused(
            "moved_into_field",
            "#include <stdlib.h>\nstruct cell { int mark; struct cell *after; };\nint main(void) {\n    struct cell *front = malloc(sizeof *front);\n    struct cell *back = malloc(sizeof *back);\n    back->after = NULL;\n    front->after = back;\n    back->mark = 9;\n    front->mark = 1;\n    int got = front->after->mark + front->mark;\n    free(front->after);\n    free(front);\n    return got - 10;\n}\n"
        ),
        reason
    );
    assert_eq!(
        refused(
            "moved_in_loop",
            "#include <stdlib.h>\nstruct cell { int mark; struct cell *after; };\nint main(void) {\n    struct cell *spare = malloc(sizeof *spare), *list = NULL;\n    spare->after = NULL;\n    for (int i = 0; i < 2; i++) {\n        list = spare;\n        spare->mark = i;\n    }\n    int got = list->mark;\n    free(list);\n    return got - 1;\n}\n"
        ),
        // The store in the loop reads the owner that the round before
        // moved.
        reason.replace("line 8", "line 7")
    );
}

/// A global owner that hands its memory to another owner is `None`
/// afterwards, where C still reads the pointer.
#[test]
fn a_global_owner_used_after_it_was_moved_follows_its_memory_or_is_refused() {
    let shelf = |take: &str| {
        format!(
            "#include <stdio.h>\n#include <stdlib.h>\nstruct plate {{ int size; struct plate *under; }};\nstruct plate *pile;\nstatic void put(int size) {{\n    struct plate *fresh = malloc(sizeof(struct plate));\n    fresh->size = size;\n    fresh->under = pile;\n    pile = fresh;\n}}\nstatic int take(void) {{\n    struct plate *gone = pile;\n{take}    free(gone);\n    return size;\n}}\nint main(void) {{\n    put(3); put(5); put(8);\n    printf(\"%d\\n\", take());\n    printf(\"%d\\n\", take());\n    free(pile);\n    return 0;\n}}\n"
        )
    };
    // The global is read after `gone` got its memory: the read names
    // `gone` (R66).
    let rust = crate::common::check(
        "global-moved",
        &shelf("    pile = pile->under;\n    int size = gone->size;\n"),
        &[&[]],
        "",
    );
    crate::common::assert_lines(
        &rust,
        &["globals.pile = gone.as_deref_mut().unwrap().under.take();"],
    );
    // A function of the program between the two may store to the global.
    assert_eq!(
        translate(
            "global-moved-call",
            &shelf("    put(1);\n    pile = pile->under;\n    int size = gone->size;\n")
        )
        .unwrap_err(),
        "unsupported: a pointer that is used after its memory went to another owner, at line 14"
    );
    // The same through the local that now owns the memory is translated.
    let rust = crate::common::check(
        "global-moved-on",
        &shelf("    int size = gone->size;\n    pile = gone->under;\n"),
        &[&[]],
        "",
    );
    crate::common::assert_lines(
        &rust,
        &["globals.pile = gone.as_deref_mut().unwrap().under.take();"],
    );
}

/// An enumerator is a constant of type `i32`, which one beyond `int` does
/// not fit.
#[test]
fn an_enumerator_beyond_int_is_refused() {
    let reason = "unsupported: an enumerator beyond `int`: no rule yet, at line 4";
    for (name, value) in [
        ("enum-unsigned", "3000000000U"),
        ("enum-sum", "2147483647U + 1"),
    ] {
        let c = format!(
            "#include <stdio.h>\nint main(void) {{\n    enum {{ NEAR = 1, FAR = {value} }} r;\n    r = FAR;\n    printf(\"%d\\n\", r == NEAR);\n    return 0;\n}}\n"
        );
        assert_eq!(translate(name, &c).unwrap_err(), reason);
    }
    // The greatest `int` is one.
    let rust = crate::common::check(
        "enum-greatest",
        "#include <stdio.h>\nenum reach { NEAR = -2147483647 - 1, FAR = 2147483647 };\nint main(void) {\n    enum reach r = FAR;\n    printf(\"%d %d\\n\", r, NEAR);\n    return 0;\n}\n",
        &[&[]],
        "",
    );
    crate::common::assert_lines(&rust, &["const FAR: i32 = 2147483647;"]);
}

/// C lets an initialiser name its own variable; `sizeof` of it reads
/// nothing and stays.
#[test]
fn a_variable_that_its_own_initialiser_names_is_refused() {
    let reason = "unsupported: a variable that its own initialiser names, at line 9";
    for (name, init) in [
        ("self-call", "struct cell *first = link(first, 4);"),
        ("self-address", "struct cell *first = link(&*first, 4);"),
    ] {
        let c = format!(
            "#include <stdio.h>\n#include <stdlib.h>\nstruct cell {{ int weight; struct cell *rest; }};\nstruct cell *link(struct cell *rest, int weight) {{\n    (void)rest;\n    struct cell *made = malloc(sizeof *made);\n    made->weight = weight;\n    made->rest = NULL;\n    return made;\n}}\nint main(void) {{\n    {init}\n    printf(\"%d\\n\", first->weight);\n    free(first);\n    return 0;\n}}\n"
        );
        let refused = translate(name, &c).unwrap_err();
        assert_eq!(refused.replace("line 12", "line 9"), reason);
    }
    let rust = crate::common::check(
        "self-sizeof",
        "#include <stdio.h>\n#include <stdlib.h>\nint main(void) {\n    long *slots = malloc(3 * sizeof *slots);\n    unsigned long width = sizeof width;\n    slots[2] = 7;\n    printf(\"%ld %lu\\n\", slots[2], width);\n    free(slots);\n    return 0;\n}\n",
        &[&[]],
        "",
    );
    crate::common::assert_lines(&rust, &["let width: u64 = 8;"]);
}

/// `p = p->next` walks on through `p` itself; any other value that reads
/// `p` would borrow it while it is assigned.
#[test]
fn a_walk_by_more_than_one_field_is_refused() {
    let reason = "unsupported: a pointer that is stored through and assigned from itself other than by one field: no rule yet, at line 6";
    for (name, step) in [
        (
            "walk-either",
            "at = far ? at->after : at;\n    printf(\"%d\\n\", at->load);",
        ),
        (
            "walk-twice",
            "at = at->after->after;\n    printf(\"%d %d\\n\", at == NULL, far);",
        ),
    ] {
        let c = format!(
            "#include <stdio.h>\n#include <stdlib.h>\nstruct knot {{ int load; struct knot *after; }};\nvoid raise(struct knot *at, int far) {{\n    at->load += 3;\n    {step}\n}}\nint main(void) {{\n    struct knot *head = malloc(sizeof(struct knot));\n    head->load = 1;\n    head->after = malloc(sizeof(struct knot));\n    head->after->load = 2;\n    head->after->after = NULL;\n    raise(head, 1);\n    raise(head, 0);\n    free(head->after);\n    free(head);\n    return 0;\n}}\n"
        );
        assert_eq!(translate(name, &c).unwrap_err(), reason);
    }
}

/// A global pointer is the place it names only while nothing changes it.
#[test]
fn a_global_pointer_that_changes_or_has_no_place_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "moved-pointer",
            "int one = 1, two = 2;\nint *at = &one;\nint main(void) {\n    at = &two;\n    return *at;\n}\n"
        ),
        "unsupported: the pointer `at`: it owns memory and also points into a variable, at line 2"
    );
    assert_eq!(
        refused(
            "taken-pointer",
            "int one = 1;\nint *at = &one;\nvoid set(int **to) { **to = 4; }\nint main(void) {\n    set(&at);\n    return one;\n}\n"
        ),
        "unsupported: the pointer `at`: it owns memory and also points into a variable, at line 2"
    );
    assert_eq!(
        refused(
            "null-pointer",
            "int *none = 0;\nint main(void) {\n    return none == 0;\n}\n"
        ),
        "unsupported: the global pointer `none` that never changes"
    );
    assert_eq!(
        refused(
            "array-literal",
            "int *row = (int[]){ 1, 2 };\nint main(void) {\n    return row[1];\n}\n"
        ),
        "unsupported: a compound literal that is an array: no rule yet, at line 1"
    );
    assert_eq!(
        refused(
            "local-literal",
            "struct two { int a; int b; };\nint main(void) {\n    struct two *t = &(struct two){ 1, 2 };\n    return t->a;\n}\n"
        ),
        "unsupported: a pointer to a compound literal: no rule yet, at line 3"
    );
}

#[test]
fn a_copy_of_a_pointer_that_is_read_after_the_pointer_went_on_is_refused() {
    // `first` would borrow `at`, which is stored through and then walks
    // on while `first` is still read.
    let c = "#include <stdio.h>\n#include <stdlib.h>\nstruct knot { int load; struct knot *after; };\nvoid raise(struct knot *at) {\n    struct knot *first = at;\n    at->load += 3;\n    at = at->after;\n    printf(\"%d %d\\n\", at->load, first->load);\n}\nint main(void) {\n    struct knot *head = malloc(sizeof(struct knot));\n    head->load = 1;\n    head->after = malloc(sizeof(struct knot));\n    head->after->load = 2;\n    head->after->after = NULL;\n    raise(head);\n    free(head->after);\n    free(head);\n    return 0;\n}\n";
    assert_eq!(
        translate("walk-copy", c).unwrap_err(),
        "unsupported: a variable that is used while a pointer to it is in use, at line 8"
    );
    // A copy of the copy borrows the pointer too, and `++` stores through
    // it as `=` does.
    let c = "#include <stdio.h>\n#include <stdlib.h>\nint main(void) {\n    int *count = malloc(sizeof *count);\n    *count = 1;\n    int *seen = count;\n    int *again = seen;\n    (*count)++;\n    printf(\"%d\\n\", *again);\n    free(count);\n    return 0;\n}\n";
    assert_eq!(
        translate("copy-copy", c).unwrap_err(),
        "unsupported: a variable that is used while a pointer to it is in use, at line 9"
    );
}

#[test]
fn a_result_taken_out_of_a_borrowed_record_is_refused() {
    // `rest` would take the next knot out of the list, which C leaves
    // linked.
    let c = "#include <stdio.h>\n#include <stdlib.h>\nstruct knot { int load; struct knot *after; };\nstruct knot *rest(struct knot *at) { return at->after; }\nvoid raise(struct knot *at) {\n    while (at) {\n        at->load += 1;\n        at = rest(at);\n    }\n}\nint main(void) {\n    struct knot *head = malloc(sizeof(struct knot));\n    head->load = 1;\n    head->after = NULL;\n    raise(head);\n    printf(\"%d\\n\", head->load);\n    free(head);\n    return 0;\n}\n";
    assert_eq!(
        translate("walk-call", c).unwrap_err(),
        "unsupported: a result that takes its memory out of a record that the function borrows: no rule yet, at line 4"
    );
    let c = c.replace("return at->after;", "return at->after->after;");
    assert_eq!(
        translate("walk-call-twice", &c).unwrap_err(),
        "unsupported: a result that takes its memory out of a record that the function borrows: no rule yet, at line 4"
    );
}
