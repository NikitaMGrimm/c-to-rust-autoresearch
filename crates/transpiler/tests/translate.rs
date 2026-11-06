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
    std::fs::remove_dir_all(&dir).unwrap();
}
