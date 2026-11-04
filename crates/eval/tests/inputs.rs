use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const ECHO_C: &str = r#"#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    char line[64] = "";
    if (fgets(line, sizeof line, stdin) == NULL) line[0] = '\0';
    printf("%s|%s", argc > 1 ? argv[1] : "-", line);
    return argc;
}
"#;

const ECHO_RS: &str = r#"fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    print!("{}|{}", args.get(1).map_or("-", String::as_str), line);
    std::process::exit(args.len() as i32);
}
"#;

const INPUTS: &str = r#"{"inputs": [
    {"name": "plain", "args": [], "stdin": ""},
    {"name": "both", "args": ["7"], "stdin": "hello\n"}
]}"#;

/// A scratch directory with `program.c`, its inputs and a stand-in transpiler
/// that writes `rust` as the translation.
fn setup(test: &str, c: &str, inputs: Option<&str>, rust: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    std::fs::write(dir.join("g1_program.c"), c).unwrap();
    if let Some(inputs) = inputs {
        std::fs::write(dir.join("g1_program.json"), inputs).unwrap();
    }
    std::fs::write(dir.join("prepared.rs"), rust).unwrap();
    let transpiler = dir.join("transpiler");
    std::fs::write(
        &transpiler,
        // Relative to the script, so the path needs no quoting.
        "#!/bin/sh\ncp \"$(dirname \"$0\")/prepared.rs\" \"$3\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&transpiler, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, transpiler)
}

fn evaluate(test: &str, c: &str, inputs: Option<&str>, rust: &str) -> eval::Record {
    let (dir, transpiler) = setup(test, c, inputs, rust);
    let record = eval::evaluate(&dir.join("g1_program.c"), &transpiler, &dir.join("work"));
    std::fs::remove_dir_all(&dir).unwrap();
    record
}

#[test]
fn translation_matching_every_input_passes() {
    let record = evaluate("inputs-pass", ECHO_C, Some(INPUTS), ECHO_RS);
    assert_eq!(record.stage, "pass", "{}", record.detail);
}

#[test]
fn translation_ignoring_stdin_fails_on_that_input() {
    let ignores_stdin = ECHO_RS.replace("std::io::stdin().read_line(&mut line).unwrap();", "");
    let record = evaluate("inputs-mismatch", ECHO_C, Some(INPUTS), &ignores_stdin);
    assert_eq!(record.stage, "mismatch");
    assert!(
        record.detail.starts_with("input both: "),
        "{}",
        record.detail
    );
}

#[test]
fn undefined_behaviour_on_one_input_invalidates_the_c_program() {
    let c = "#include <stdlib.h>\nint main(int argc, char **argv) {\n    int a[4] = {0};\n    return argc > 1 ? a[atoi(argv[1])] : 0;\n}\n";
    let record = evaluate("inputs-ub", c, Some(INPUTS), ECHO_RS);
    assert_eq!(record.stage, "c_invalid");
    assert!(
        record.detail.starts_with("input both: "),
        "{}",
        record.detail
    );
}

#[test]
fn malformed_inputs_file_is_reported() {
    let record = evaluate(
        "inputs-malformed",
        ECHO_C,
        Some(r#"{"inputs": []}"#),
        ECHO_RS,
    );
    assert_eq!(record.stage, "c_invalid");
    assert!(record.detail.contains("non-empty"), "{}", record.detail);
}

#[test]
fn input_that_cannot_be_passed_is_not_a_pass() {
    let nul = r#"{"inputs": [{"name": "nul", "args": ["a\u0000b"]}]}"#;
    let record = evaluate("inputs-nul", ECHO_C, Some(nul), ECHO_RS);
    assert_eq!(record.stage, "c_invalid", "{}", record.detail);
}

#[test]
fn unsafe_is_reported_after_a_mismatch_on_a_later_input() {
    let unsafe_ignoring_stdin = ECHO_RS
        .replace("std::io::stdin().read_line(&mut line).unwrap();", "")
        .replace(
            "fn main() {",
            "fn main() {\n    let x = 1;\n    let _ = unsafe { *&raw const x };",
        );
    let record = evaluate(
        "inputs-unsafe",
        ECHO_C,
        Some(INPUTS),
        &unsafe_ignoring_stdin,
    );
    assert_eq!(record.stage, "mismatch", "{}", record.detail);
}

#[test]
fn rust_panic_fails_even_when_c_exits_with_101() {
    let c = "int main(void) { return 101; }\n";
    let record = evaluate(
        "inputs-101",
        c,
        None,
        "fn main() {\n    panic!(\"boom\");\n}\n",
    );
    assert_eq!(record.stage, "panic", "{}", record.detail);
}
