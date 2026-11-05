use eval::Stage;
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

// The failing input comes first, so a failure must stop at it and name it.
const INPUTS: &str = r#"{"inputs": [
    {"name": "both", "args": ["7"], "stdin": "hello\n"},
    {"name": "plain", "args": [], "stdin": ""}
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
    // The sandbox hides everything next to the transpiler, so it carries the translation.
    std::fs::write(
        &transpiler,
        format!("#!/bin/sh\ncat > \"$3\" <<'RUST'\n{rust}RUST\n"),
    )
    .unwrap();
    std::fs::set_permissions(&transpiler, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, transpiler)
}

fn evaluate(test: &str, c: &str, inputs: Option<&str>, rust: &str) -> eval::Record {
    let (dir, transpiler) = setup(test, c, inputs, rust);
    let record = eval::evaluate(
        &dir.join("g1_program.c"),
        &eval::Translator::Transpiler(transpiler),
        &dir.join("work"),
    );
    std::fs::remove_dir_all(&dir).unwrap();
    record
}

#[test]
fn translation_matching_every_input_passes() {
    let record = evaluate("inputs-pass", ECHO_C, Some(INPUTS), ECHO_RS);
    assert_eq!(record.stage, Stage::Pass, "{}", record.detail);
}

#[test]
fn translation_ignoring_stdin_fails_on_that_input() {
    let ignores_stdin = ECHO_RS.replace("std::io::stdin().read_line(&mut line).unwrap();", "");
    let record = evaluate("inputs-mismatch", ECHO_C, Some(INPUTS), &ignores_stdin);
    assert_eq!(record.stage, Stage::Mismatch);
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
    assert_eq!(record.stage, Stage::CInvalid);
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
    assert_eq!(record.stage, Stage::CInvalid);
    assert!(record.detail.contains("non-empty"), "{}", record.detail);
}

#[test]
fn input_that_cannot_be_passed_is_not_a_pass() {
    let nul = r#"{"inputs": [{"name": "nul", "args": ["a\u0000b"]}]}"#;
    let record = evaluate("inputs-nul", ECHO_C, Some(nul), ECHO_RS);
    assert_eq!(record.stage, Stage::CInvalid, "{}", record.detail);
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
    assert_eq!(record.stage, Stage::Mismatch, "{}", record.detail);
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
    assert_eq!(record.stage, Stage::Panic, "{}", record.detail);
}

#[test]
fn c_and_rust_programs_see_the_same_argv0() {
    let c = "#include <stdio.h>\nint main(int argc, char **argv) { (void)argc; puts(argv[0]); return 0; }\n";
    let rust = "fn main() {\n    println!(\"{}\", std::env::args().next().unwrap());\n}\n";
    let record = evaluate("argv0", c, None, rust);
    assert_eq!(record.stage, Stage::Pass, "{}", record.detail);
}

#[test]
fn c_output_beyond_the_limit_is_invalid() {
    let c = "#include <stdio.h>\nint main(void) { for (int i = 0; i < (9 << 20) / 64; i++) printf(\"%063d\\n\", i); return 0; }\n";
    let record = evaluate("output-limit", c, None, ECHO_RS);
    assert_eq!(record.stage, Stage::CInvalid);
    assert!(
        record.detail.contains("output exceeds"),
        "{}",
        record.detail
    );
}

#[test]
fn translation_beyond_the_memory_limit_fails() {
    let c = "#include <stdio.h>\nint main(void) { puts(\"0\"); return 0; }\n";
    // Zeroed pages are never touched, so this cannot exhaust memory if the limit is missing.
    let rust = "fn main() {\n    let big = vec![0u8; 8 << 30];\n    println!(\"{}\", big[big.len() - 1]);\n}\n";
    let record = evaluate("memory-limit", c, None, rust);
    assert_ne!(record.stage, Stage::Pass, "{}", record.detail);
}

#[test]
fn existing_translations_are_evaluated_without_the_transpiler() {
    let (dir, _) = setup("existing", ECHO_C, Some(INPUTS), ECHO_RS);
    let existing = dir.join("translations");
    std::fs::create_dir(&existing).unwrap();
    std::fs::copy(dir.join("prepared.rs"), existing.join("g1_program.rs")).unwrap();
    let c_file = dir.join("g1_program.c");
    let found = eval::evaluate(
        &c_file,
        &eval::Translator::Existing(existing),
        &dir.join("work"),
    );
    let missing = eval::evaluate(
        &c_file,
        &eval::Translator::Existing(dir.join("none")),
        &dir.join("work2"),
    );
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(found.stage, Stage::Pass, "{}", found.detail);
    assert_eq!(missing.stage, Stage::TranspileFail);
}

#[test]
fn only_known_c_flags_are_accepted() {
    let with = |flags: &str| format!(r#"{{"c_flags": {flags}, "inputs": [{{"name": "plain"}}]}}"#);
    let allowed = evaluate(
        "flags-ok",
        ECHO_C,
        Some(&with(r#"["-fno-strict-aliasing"]"#)),
        ECHO_RS,
    );
    assert_eq!(allowed.stage, Stage::Pass, "{}", allowed.detail);
    let other = evaluate("flags-bad", ECHO_C, Some(&with(r#"["-O3"]"#)), ECHO_RS);
    assert_eq!(other.stage, Stage::CInvalid);
    assert!(other.detail.contains("c_flags"), "{}", other.detail);
}

/// Only a scratch directory under $HOME, the usual case, is hidden from agent code.
fn scratch_is_hidden() -> bool {
    std::env::var_os("HOME")
        .is_some_and(|home| Path::new(env!("CARGO_TARGET_TMPDIR")).starts_with(home))
}

#[test]
fn agent_code_cannot_read_home_keep_files_or_connect() {
    let rust = r#"fn main() {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.ancestors().nth(3).unwrap();
    let _ = std::fs::write(dir.join("escaped-translation"), "");
    let read = std::fs::read(dir.join("g1_program.c")).is_ok();
    let address = std::net::SocketAddr::from(([1, 1, 1, 1], 53));
    let timeout = std::time::Duration::from_secs(2);
    let connected = std::net::TcpStream::connect_timeout(&address, timeout).is_ok();
    println!("{read} {connected}");
}
"#;
    let hidden = scratch_is_hidden();
    let c = format!(
        "#include <stdio.h>\nint main(void) {{ puts(\"{} false\"); return 0; }}\n",
        !hidden
    );
    let (dir, transpiler) = setup("sandbox", &c, None, rust);
    let script = std::fs::read_to_string(&transpiler).unwrap();
    let script = script.replacen(
        "\n",
        "\ntouch \"$(dirname \"$0\")/escaped-transpiler\"\n",
        1,
    );
    std::fs::write(&transpiler, script).unwrap();
    let record = eval::evaluate(
        &dir.join("g1_program.c"),
        &eval::Translator::Transpiler(transpiler),
        &dir.join("work"),
    );
    let escaped = ["escaped-translation", "escaped-transpiler"].map(|name| dir.join(name).exists());
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(record.stage, Stage::Pass, "{}", record.detail);
    assert_eq!(escaped, [false, false]);
}

#[test]
fn translations_cannot_include_hidden_files_when_compiled() {
    let c = "#include <stdio.h>\nint main(void) { puts(\"0\"); return 0; }\n";
    // The translation lies in work/rust; the C source two levels up.
    let rust =
        "fn main() {\n    println!(\"{}\", include_str!(\"../../g1_program.c\").len());\n}\n";
    let record = evaluate("sandbox-include", c, None, rust);
    if scratch_is_hidden() {
        assert_eq!(record.stage, Stage::CompileFail, "{}", record.detail);
    }
}

#[test]
fn fuzzed_inputs_are_a_separate_result_for_passes() {
    let c = "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char **argv) {\n    puts(argc > 1 && atoi(argv[1]) > 10 ? \"big\" : \"small\");\n    return 0;\n}\n";
    let inputs = r#"{"inputs": [{"name": "five", "args": ["5"]}]}"#;
    let fuzzed = r#"{"inputs": [{"name": "fuzz_1", "args": ["50"]}]}"#;
    let right = "fn main() {\n    let n: i32 = std::env::args().nth(1).map_or(0, |a| a.parse().unwrap_or(0));\n    println!(\"{}\", if n > 10 { \"big\" } else { \"small\" });\n}\n";
    let always_small = "fn main() {\n    println!(\"small\");\n}\n";
    let broken = "fn main() {\n    println!(\"other\");\n}\n";
    let run = |test: &str, rust: &str| {
        let (dir, transpiler) = setup(test, c, Some(inputs), rust);
        std::fs::write(dir.join("g1_program.fuzz.json"), fuzzed).unwrap();
        let record = eval::evaluate(
            &dir.join("g1_program.c"),
            &eval::Translator::Transpiler(transpiler),
            &dir.join("work"),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        record
    };
    let held = run("fuzz-right", right);
    let lucky = run("fuzz-lucky", always_small);
    let failed = run("fuzz-broken", broken);
    assert_eq!(
        (held.stage, held.fuzz.map(|f| f.0)),
        (Stage::Pass, Some(Stage::Pass))
    );
    assert_eq!(lucky.stage, Stage::Pass, "{}", lucky.detail);
    assert!(lucky.fails_fuzz());
    let (stage, detail) = lucky.fuzz.unwrap();
    assert_eq!(stage, Stage::Mismatch);
    assert!(detail.starts_with("input fuzz_1: "), "{detail}");
    assert_eq!((failed.stage, failed.fuzz), (Stage::Mismatch, None));
}
