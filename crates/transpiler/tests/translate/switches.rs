use crate::common::{assert_lines, check, translate};

const RUNS: &[&[&str]] = &[
    &["-1"],
    &["0"],
    &["1"],
    &["2"],
    &["3"],
    &["4"],
    &["5"],
    &["6"],
];

#[test]
fn cases_that_fall_through_run_for_the_labels_before_them() {
    let rust = check(
        "fall-through",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int count = 0;
static int next(int x) { count++; return x; }
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    int x = atoi(argv[1]);
    int r = 0;
    switch (next(x)) {
    case 1: r += 1;
        __attribute__((fallthrough));
    case 2: case 3: r += 10; x = 4;
        /* fall through */
    case 4:
        r += 100;
        if (r > 100) break;
        r += 1000;
    case 0: break;
    case 5: r = 5;
    }
    printf("%d %d %d\n", r, x, count);
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "'switch: {",
            "entry @ (1 | 2 | 3 | 4 | 0) => {",
            "if entry == 1 {",
            "if matches!(entry, 1 | 2 | 3) {",
            "if matches!(entry, 1 | 2 | 3 | 4) {",
            "break 'switch;",
            "5 => {",
        ],
    );
}

#[test]
fn a_default_that_falls_through_runs_for_all_but_the_labels_after_it() {
    let rust = check(
        "fall-through-default",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int first(int x) {
    int r = 0;
    switch (x) {
    default: r += 1;
    case 2: r += 10;
    case 3: r += 100;
        break;
    case 4: r = 9;
    }
    return r;
}
static int middle(int x) {
    int r = 0;
    switch (x) {
    case 1: r += 1;
    default: r += 10;
    case 3: r += 100;
        break;
    case 4: r = 9; break;
    }
    return r;
}
static int last(int x) {
    int r = 0;
    switch (x) {
    case 1: r += 1;
    case 2: r += 10;
    default: r += 100;
    }
    return r;
}
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    int x = atoi(argv[1]);
    printf("%d %d %d\n", first(x), middle(x), last(x));
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(
        &rust,
        &[
            "4 => {",
            "entry => {",
            "if !matches!(entry, 2 | 3) {",
            "if entry != 3 {",
            "if entry == 1 {",
            "if matches!(entry, 1 | 2) {",
        ],
    );
}

#[test]
fn a_case_that_falls_through_returns_and_continues_as_in_c() {
    let rust = check(
        "fall-through-leave",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int pick(int x, int y) {
    int r = 0;
    switch (x) {
    case 1:
        r += 1;
        if (y) return -r;
    case 2: {
        int k = r * 3;
        r += k + 10;
    }
    case 3:
        switch (y) {
        case 0: r += 2;
        case 1: r += 3; break;
        }
        for (int i = 0; i < 5; i++) {
            if (i == y) break;
            r += i;
        }
        return r;
    }
    return r + 1;
}
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    int x = atoi(argv[1]), total = 0;
    for (int y = 0; y < 3; y++) {
        switch (x + y) {
        case 2: total += 1;
            if (y == 1) continue;
        case 3: total += 10;
            continue;
        }
        printf("%d %d\n", pick(x, y), total);
    }
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(&rust, &["entry @ (1 | 2 | 3) => {", "entry @ (2 | 3) => {"]);
}

#[test]
fn a_c_name_entry_does_not_meet_the_bound_value() {
    let rust = check(
        "fall-through-entry",
        r#"
#include <stdio.h>
#include <stdlib.h>
static int pick(int entry) {
    int r = 0;
    switch (entry) {
    case 1: r += 1; entry = 7;
    case 2: r += entry;
        break;
    }
    return r + entry;
}
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    printf("%d\n", pick(atoi(argv[1])));
    return 0;
}
"#,
        RUNS,
        "",
    );
    assert_lines(&rust, &["entry @ (1 | 2) => {", "entry_ = 7;"]);
}

#[test]
fn a_variable_that_a_later_case_uses_is_refused() {
    assert_eq!(
        translate(
            "fall-through-declared",
            "int main(int argc, char **argv) {\n    int r = 0;\n    switch (argc) {\n    case 1: ;\n        int k = 5;\n        r += k;\n    case 2:\n        k = 7;\n        r += k;\n        break;\n    }\n    return r;\n}\n",
        )
        .unwrap_err(),
        "unsupported: a variable that a `case` declares and a later `case` uses: no rule yet, at line 3"
    );
    // C's scope goes on past the `break`, into cases that are other arms.
    assert_eq!(
        translate(
            "fall-through-declared-later",
            "int main(int argc, char **argv) {\n    int k = argc, r = 0;\n    switch (argc) {\n    case 1: r += 1;\n    case 2: ;\n        int k = 5;\n        r += k;\n        break;\n    case 3:\n        k = 9;\n        r += k;\n        break;\n    }\n    return r + k;\n}\n",
        )
        .unwrap_err(),
        "unsupported: a variable that a `case` declares and a later `case` uses: no rule yet, at line 3"
    );
    // Rust wants a label on a `continue` that leaves a block with a label.
    assert_eq!(
        translate(
            "fall-through-continue",
            "int main(int argc, char **argv) {\n    int n = 0;\n    while (n < 5) {\n        n++;\n        switch (n) {\n        case 1:\n            if (argc) break;\n        case 2:\n            continue;\n        }\n    }\n    return n;\n}\n",
        )
        .unwrap_err(),
        "unsupported: `continue` in a `switch` with a `break` in the middle of a `case`: no rule yet, at line 5"
    );
    // Duff's device puts a label inside a loop.
    assert_eq!(
        translate(
            "fall-through-duff",
            "int main(int argc, char **argv) {\n    int n = 2, total = 0;\n    switch (argc % 2) {\n    case 0: do { total += 1;\n    case 1:      total += 2;\n            } while (--n > 0);\n    }\n    return total;\n}\n",
        )
        .unwrap_err(),
        "unsupported: a `case` outside the top of its `switch`"
    );
}
