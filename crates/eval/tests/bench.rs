use std::path::Path;
use std::process::Command;

#[test]
fn evaluates_a_benchmark_directory() {
    // Concurrent test runs share CARGO_TARGET_TMPDIR, so each process gets its own directory.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "evaluates_a_benchmark_directory-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let bench = dir.join("bench");
    std::fs::create_dir_all(&bench).unwrap();
    std::fs::write(
        bench.join("g1_hello.c"),
        "#include <stdio.h>\nint main(void) { puts(\"hi\"); return 0; }\n",
    )
    .unwrap();
    std::fs::write(
        bench.join("g1_overflow.c"),
        "#include <limits.h>\nint main(int argc, char **argv) { (void)argv; int x = INT_MAX; return x + argc; }\n",
    )
    .unwrap();
    let json = dir.join("results.json");

    let output = Command::new(env!("CARGO_BIN_EXE_eval"))
        .arg(&bench)
        .arg("--json")
        .arg(&json)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(results["total"], 2);
    assert_eq!(results["passed"], 0);
    // The skeleton transpiler emits an empty file, which has no main function.
    assert_eq!(results["results"][0]["name"], "g1_hello");
    assert_eq!(results["results"][0]["stage"], "compile_fail");
    assert_eq!(results["results"][1]["stage"], "c_invalid");
    std::fs::remove_dir_all(&dir).unwrap();
}
