use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A fresh directory for one test, since concurrent runs share CARGO_TARGET_TMPDIR.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn translate(dir: &Path, c: &str) -> String {
    let input = dir.join("input.c");
    std::fs::write(&input, c).unwrap();
    transpiler::translate(&transpiler::clang_ast(&input).unwrap())
}

fn build(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run(program: &Path, stdin: &str) -> Output {
    let mut child = Command::new(program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Checks that the translation of `c` prints and exits as the C program does
/// on each standard input, with overflow checks on as in the evaluator.
fn same_behaviour(test: &str, c: &str, stdins: &[&str]) -> String {
    let dir = scratch(test);
    let rust = translate(&dir, c);
    std::fs::write(dir.join("translation.rs"), &rust).unwrap();
    build(
        Command::new(transpiler::CLANG)
            .arg(dir.join("input.c"))
            .arg("-o")
            .arg(dir.join("c")),
    );
    build(
        Command::new("rustc")
            .args(["--edition", "2024", "-C", "overflow-checks=on", "-o"])
            .arg(dir.join("rust"))
            .arg(dir.join("translation.rs")),
    );
    for stdin in stdins {
        let (c, rust) = (run(&dir.join("c"), stdin), run(&dir.join("rust"), stdin));
        assert_eq!(
            String::from_utf8_lossy(&rust.stdout),
            String::from_utf8_lossy(&c.stdout),
            "stdin {stdin:?}"
        );
        assert_eq!(rust.status.code(), c.status.code(), "stdin {stdin:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
    rust
}

#[test]
fn translates_int_functions_and_formatted_io() {
    let c = r#"
#include <stdio.h>
#include <stdlib.h>

int scale(int a, int b) {
    int c = a * 3 - b / 2;
    c %= 7;
    return -c + (a - b) * 2;
}

void show(int type) {
    type -= 1;
    printf("[%d] {%5d|%-4d|%+d|%03d} 100%%\n", type, type, type, type, type);
}

int main(void) {
    int x = 4, y;
    int n = scanf("%d %d", &x, &y);
    show(scale(x, n));
    printf("tab\there \"quoted\" \\ \101\x42 no newline");
    return n + 2;
}
"#;
    let rust = same_behaviour(
        "translates_int_functions_and_formatted_io",
        c,
        &[
            "17 -5",
            "  -3\n\t+12",
            "8",
            "",
            "x",
            "-",
            "-x 3",
            "99999999999999999999 1",
        ],
    );
    // glibc's inline helpers from stdlib.h are not part of the program.
    assert!(!rust.contains("bswap"), "{rust}");
    assert!(rust.contains("mut r#type: i32"), "{rust}");
}

#[test]
fn reports_unsupported_constructs_as_compile_errors() {
    let dir = scratch("reports_unsupported_constructs_as_compile_errors");
    let rust = translate(
        &dir,
        "#include <stdio.h>\nint main(void) { printf(\"%f\\n\", 1.5); return 0; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: printf conversion %f")"#),
        "{rust}"
    );
    // A block-scope extern refers to glibc's optind, which is 1, not 0.
    let rust = translate(
        &dir,
        "int main(void) { extern int optind; return optind; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: extern local variable")"#),
        "{rust}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_control_flow_and_goto() {
    let c = r#"
#include <stdio.h>

int seen(int v) {
    printf("seen %d\n", v);
    return v;
}

int loops(int n) {
    int sum = 0;
    for (int i = 0; i < n; i++) {
        if (i % 3 == 0)
            continue;
        for (int j = 0;; j++) {
            if (j >= i)
                break;
            sum += j;
        }
        if (sum > 40)
            break;
    }
    int k = n;
    do {
        k--;
        if (k % 2)
            continue;
        sum += k;
    } while (k > 0);
    while (1) {
        if (!(sum < 100) || n == 0)
            break;
        sum += n;
    }
    return sum;
}

int jumps(int x, int y) {
    unsigned char wrap = 250;
    while (x > 0 || y > 0) {
        printf("x %d y %d\n", x, y);
        if (x == 1 && y == 4) {
            goto skip;
        }
    again:
        if (x > 0) {
            x--;
        }
    skip:
        wrap++;
        if (y == 0) {
            continue;
        }
        y--;
        for (int i = 0; i < 3; i++) {
            if (i == x)
                goto again;
        }
    }
    if (x == 0) goto out;
    printf("unreachable\n");
out:
    return wrap;
}

int main(void) {
    int n = 0, m = 0;
    scanf("%d %d", &n, &m);
    int t = (n < m) + (n == m) * 2 - !n;
    if (n > 5) {
        printf("big\n");
    } else if (seen(n) && seen(m) || seen(-1)) {
        printf("both\n");
    } else {
        printf("neither\n");
    }
    printf("%d %d %d\n", t, loops(n), jumps(n, m));
    return 0;
}
"#;
    let rust = same_behaviour(
        "translates_control_flow_and_goto",
        c,
        &["0 0", "1 4", "2 5", "4 1", "7 3", "3 3", "0 9"],
    );
    assert!(rust.contains("__goto"), "{rust}");
}

#[test]
fn rejects_jumps_it_cannot_translate() {
    let dir = scratch("rejects_jumps_it_cannot_translate");
    let rust = translate(
        &dir,
        "int main(void) { int x = 1; goto in; if (x) { in: x = 2; } return x; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: goto into a block")"#),
        "{rust}"
    );
    // Declaring the inner x before the block would hide the outer one that
    // the block assigns first.
    let rust = translate(
        &dir,
        "int main(void) { int x = 1; { again: x = x + 1; int x = 5;\n\
         if (x < 0) goto again; } return x; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: declaration in a block with labels")"#),
        "{rust}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
