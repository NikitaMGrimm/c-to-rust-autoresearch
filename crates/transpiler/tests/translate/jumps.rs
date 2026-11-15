use crate::common::{assert_lines, check, translate};

/// R48: a `goto` to a label that follows is a `break` of the block that
/// ends at the label.
#[test]
fn a_goto_to_a_label_that_follows_leaves_a_block() {
    let runs: &[&[&str]] = &[&["0"], &["1"], &["2"], &["7"]];
    let rust = check(
        "goto_cleanup",
        r#"#include <stdio.h>
#include <stdlib.h>

static int work(int n) {
    int result = 0;
    int *data = NULL;
    if (n == 0) {
        printf("nothing\n");
        goto cleanup;
    }
    data = malloc(sizeof(int));
    if (!data) goto cleanup;
    *data = n * 3;
    if (n > 5) {
        result = -1;
        goto cleanup;
    }
    result = *data + 1;
cleanup:
    free(data);
    return result;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    printf("%d\n", work(atoi(argv[1])));
    return 0;
}
"#,
        runs,
        "",
    );
    assert_lines(&rust, &["'cleanup: {", "break 'cleanup;", "drop(data);"]);
    let rust = check(
        "goto_loops",
        r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    int found = -1;
    for (int i = 0; i < 4; i++) {
        for (int j = 0; j < 4; j++) {
            if (i * j == n) {
                found = i * 10 + j;
                goto done;
            }
        }
    }
    printf("not found\n");
done:
    printf("%d\n", found);
    return 0;
}
"#,
        runs,
        "",
    );
    assert_lines(&rust, &["'done: {", "break 'done;"]);
    // A label inside another's block, and names that Rust keeps.
    let rust = check(
        "goto_keyword",
        r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    if (n == 0) goto switch_1;
    if (n == 1) goto loop;
    printf("a\n");
loop:
    printf("b\n");
switch_1:
    printf("c\n");
    return 0;
}
"#,
        runs,
        "",
    );
    assert_lines(&rust, &["'switch_1_label: {", "'loop_label: {"]);
    check(
        "goto_chain",
        r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    int code = 0;
    if (n == 0) goto fail_first;
    printf("first ok\n");
    if (n == 1) goto fail_second;
    printf("second ok\n");
    code = 5;
fail_second:
    printf("undo first\n");
fail_first:
    printf("end %d\n", code);
    return code;
}
"#,
        runs,
        "",
    );
    check(
        "goto_inner",
        r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    for (int i = 0; i < 3; i++) {
        if (i == n) goto next;
        printf("i %d\n", i);
    next:
        printf("step\n");
    }
    return 0;
}
"#,
        runs,
        "",
    );
    check(
        "goto_end",
        r#"#include <stdio.h>
#include <stdlib.h>

static void say(int n) {
    if (n == 0) goto end;
    printf("say %d\n", n);
end:
    ;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    say(atoi(argv[1]));
    return 0;
}
"#,
        runs,
        "",
    );
}

/// A jump to a label under a `case`, out of a `switch` into a block entered
/// at a label, into a loop, and a block that Rust could not leave or that
/// would end a local too early have no rule.
#[test]
fn a_goto_that_leaves_no_block_is_refused() {
    for (name, c, refusal) in [
        (
            "goto_back",
            r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    switch (n) {
    case 1:
    again:
        printf("%d\n", n);
        n--;
        if (n > 0) goto again;
    }
    return 0;
}
"#,
            "unsupported: `goto again` to a label under a `case`: no rule yet",
        ),
        (
            "goto_both",
            r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    int tries = 0;
retry:
    tries++;
    if (n == 0) goto out;
    switch (tries < n) {
    case 1:
        goto retry;
    }
    printf("tries %d\n", tries);
out:
    printf("done\n");
    return 0;
}
"#,
            "unsupported: `goto retry` from a `switch` into a block entered at a label: no rule yet",
        ),
        (
            "goto_into",
            r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    if (n == 0) goto inside;
    while (n > 0) {
        printf("loop\n");
    inside:
        n -= 3;
    }
    return 0;
}
"#,
            "unsupported: `goto inside` into a block from outside it: no rule yet",
        ),
        (
            "goto_continue",
            r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    for (int i = 0; i < 4; i++) {
        if (i == n) goto skip;
        if (i == 1) continue;
        printf("i %d\n", i);
    skip:
        printf("s\n");
    }
    return 0;
}
"#,
            "unsupported: `break` or `continue` of a loop between `goto skip` and its label: no rule yet",
        ),
        (
            "goto_declared",
            r#"#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    if (n == 0) goto out;
    int twice = n * 2;
    printf("%d\n", twice);
out:
    twice = 1;
    printf("%d\n", twice);
    return 0;
}
"#,
            "unsupported: a variable declared between `goto out` and its label and used after it: no rule yet",
        ),
    ] {
        assert_eq!(translate(name, c).unwrap_err(), refusal, "{name}");
    }
}

/// R49: a block whose labels a `goto` reaches from below, or past another
/// label, is a loop that is entered at the part of the label.
#[test]
fn a_goto_back_enters_a_block_at_its_label() {
    let runs: &[&[&str]] = &[&["0"], &["1"], &["2"], &["3"], &["5"], &["9"]];
    let rust = check(
        "goto_back_example",
        r#"#include <stdio.h>
#include <stdlib.h>
static void walk(int x, int y) {
    while (x > 0 || y > 0) {
        printf("loop %d %d\n", x, y);
        if (x == 1 && y == 4) goto two;
    one:
        if (x > 0) x--;
    two:
        if (y == 0) continue;
        y--;
        if (x < 3) goto one;
    }
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    walk(n % 5, n / 2 + 1);
    walk(1, 4);
    return 0;
}
"#,
        runs,
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut entry = 0;",
            "'entry: loop {",
            "if entry == 0 {",
            "if entry <= 1 {",
            "entry = 2;",
            "entry = 1;",
            "continue 'entry;",
            "continue 'outer;",
            "break;",
        ],
    );
    assert!(rust.contains("'outer: while x > 0 || y > 0 {"), "{rust}");
    // At the top of a function: one part, which needs no `entry`.
    let rust = check(
        "goto_back_retry",
        r#"#include <stdio.h>
#include <stdlib.h>
static int step(int n) {
    int tries = 0;
retry:
    tries++;
    n = n / 2;
    if (n > 1) goto retry;
    return tries;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    printf("%d\n", step(atoi(argv[1])));
    return 0;
}
"#,
        runs,
        "",
    );
    assert_lines(&rust, &["'entry: loop {", "continue 'entry;"]);
    assert!(!rust.contains("let mut entry"), "{rust}");
    // Jumps forward past another label, and a forward label beside a jump
    // back.
    check(
        "goto_back_cross",
        r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    if (n == 1) goto second;
    if (n == 2) goto first;
    printf("start\n");
first:
    printf("first\n");
    if (n == 0) goto last;
second:
    printf("second\n");
last:
    printf("last\n");
    return 0;
}
"#,
        runs,
        "",
    );
    check(
        "goto_back_mixed",
        r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    int tries = 0;
    if (n == 0) goto out;
retry:
    tries++;
    if (tries < n) goto retry;
    printf("tries %d\n", tries);
out:
    printf("done %d\n", tries);
    return 0;
}
"#,
        runs,
        "",
    );
    // `break` and `continue` of the loops around, each of its own depth,
    // and a C label that would be a loop's.
    let rust = check(
        "goto_back_loops",
        r#"#include <stdio.h>
#include <stdlib.h>
static int scan(int n) {
    for (int i = 0; i < 3; i++) {
        for (int j = 0; j < 3; j++) {
            if (i + j == n + 2) goto outer;
            printf("%d %d\n", i, j);
        }
    }
outer:
    return n;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = scan(atoi(argv[1]));
    while (n < 9) {
        n++;
        for (int k = 0; k < 4; k++) {
        again:
            if (k == n % 4) break;
            printf("k %d\n", k);
            k += 2;
            if (k < 3) goto again;
        }
        if (n == 5) break;
    again2:
        n += 2;
        if (n == 4) continue;
        if (n % 3 == 0) goto again2;
    }
    printf("%d\n", n);
    return 0;
}
"#,
        runs,
        "",
    );
    assert_lines(
        &rust,
        &["'outer_label: {", "break 'outer_2;", "continue 'outer;"],
    );
    // A `switch` around, `return` in parts, and statements before the first
    // jump that run once.
    let rust = check(
        "goto_back_switch",
        r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    switch (n) {
    case 1:
    case 2: {
        int k = n;
    back:
        if (k > 5) break;
        printf("k %d\n", k);
        k *= 2;
        goto back;
    }
    default:
        printf("other\n");
        break;
    }
    return 0;
}
"#,
        runs,
        "",
    );
    assert_lines(&rust, &["break 'switch;"]);
    let rust = check(
        "goto_back_returns",
        r#"#include <stdio.h>
#include <stdlib.h>
static int find(int n) {
    int i = 0;
next:
    if (i * i == n) return i;
    if (i * i > n) {
        return -1;
    } else {
        i++;
        goto next;
    }
}
static int count(int n) {
    int c = 0;
again:
    if (n <= 0) {
        return c;
    }
    c++;
    n -= 2;
    goto again;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    printf("%d %d\n", find(n), count(n));
    return 0;
}
"#,
        runs,
        "",
    );
    assert!(!rust.contains("break;"), "{rust}");
    let rust = check(
        "goto_back_once",
        r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    int sum = 0;
    int *p = &sum;
loop:
    *p += n;
    n--;
    if (n > 0) goto loop;
    printf("%d\n", sum);
    return sum % 7;
}
"#,
        runs,
        "",
    );
    assert_lines(&rust, &["let mut sum: i32 = 0;", "'entry: loop {"]);
    // Loops of a part keep their own `break` and `continue`.
    let rust = check(
        "goto_back_inner",
        r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    int round = 0;
    while (round < 4) {
        round++;
        if (round == n) goto second;
    first:
        for (int i = 0; i < 5; i++) {
            if (i == 3) break;
            if (i == 1) continue;
            printf("i %d\n", i);
        }
        int j = 0;
        while (j < 3) {
            j++;
            if (j == 2) continue;
            printf("j %d\n", j);
        }
    second:
        if (round == 2) continue;
        printf("round %d\n", round);
        if (round == 3 && n > 2) { n = 0; goto first; }
        if (round > n + 2) break;
    }
    printf("end %d\n", round);
    return 0;
}
"#,
        runs,
        "",
    );
    assert_lines(&rust, &["continue 'outer;", "break 'outer;", "continue;"]);
    check(
        "goto_back_guard",
        r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    int *p = NULL;
    int box = 7;
again:
    if (!p) {
        printf("null\n");
        p = &box;
        goto again;
    }
    printf("%d\n", *p + n);
    return 0;
}
"#,
        runs,
        "",
    );
    check(
        "goto_back_retest",
        r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    int done = 0;
again:
    if (done) {
        printf("done %d\n", n);
        return 0;
    }
    printf("first %d\n", n);
    done = 1;
    n *= 2;
    goto again;
}
"#,
        runs,
        "",
    );
}

/// A local that ends with its part, a jump over an initialiser, a block
/// entered at a label inside another and a computed `goto` have no rule.
#[test]
fn a_goto_back_that_no_part_can_take_is_refused() {
    for (name, c, refusal) in [
        (
            "goto_back_declared",
            r#"#include <stdio.h>
#include <stdlib.h>
static int get(int n) {
retry:
    n--;
    int m = n * 3;
    if (m > 10) goto retry;
    return m;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    printf("%d\n", get(atoi(argv[1]) + 5));
    return 0;
}
"#,
            "unsupported: a variable declared in one part of a block entered at a label and used in a later one or after it: no rule yet",
        ),
        (
            "goto_back_part",
            r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
again:
    n--;
    int left;
    left = n * 2;
next:
    printf("%d\n", left);
    if (n > 0) goto again;
    if (n == -1) { n = -2; goto next; }
    return 0;
}
"#,
            "unsupported: a variable declared in one part of a block entered at a label and used in a later one or after it: no rule yet",
        ),
        (
            "goto_back_first",
            r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    if (n > 5) goto again;
    int twice;
    twice = n * 2;
again:
    printf("%d\n", twice);
    n--;
    twice = n;
    if (n > 0) goto again;
    return 0;
}
"#,
            "unsupported: a variable declared in one part of a block entered at a label and used in a later one or after it: no rule yet",
        ),
        (
            "goto_back_over",
            r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
again:
    if (n == 0) goto use;
    int twice = n * 2;
use:
    printf("%d\n", n);
    if (n > 0) { n--; goto again; }
    twice = 0;
    printf("%d\n", twice);
    return 0;
}
"#,
            "unsupported: a `goto` over a declaration with an initialiser into its scope: no rule yet",
        ),
        (
            "goto_back_nested",
            r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
outer_again:
    n++;
    if (n % 2) {
        int k = 0;
    inner:
        k++;
        if (k < 3) goto inner;
        printf("k %d\n", k);
    }
    if (n < 5) goto outer_again;
    printf("%d\n", n);
    return 0;
}
"#,
            "unsupported: a block entered at a label inside another: no rule yet",
        ),
        (
            "goto_back_computed",
            r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int n = atoi(argv[1]);
    void *where = n ? &&one : &&two;
    goto *where;
one:
    printf("one\n");
two:
    printf("two\n");
    return 0;
}
"#,
            "unsupported: a computed `goto`: no rule yet, at line 6",
        ),
    ] {
        assert_eq!(translate(name, c).unwrap_err(), refusal, "{name}");
    }
}
