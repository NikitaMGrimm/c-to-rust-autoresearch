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

/// A jump back, a jump into a loop and a block that Rust could not leave
/// or that would end a local too early have no rule.
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
again:
    printf("%d\n", n);
    n--;
    if (n > 0) goto again;
    return 0;
}
"#,
            "unsupported: `goto again`: no rule yet",
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
    if (tries < n) goto retry;
    printf("tries %d\n", tries);
out:
    printf("done\n");
    return 0;
}
"#,
            "unsupported: `goto retry`: no rule yet",
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
            "unsupported: `goto inside`: no rule yet",
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
