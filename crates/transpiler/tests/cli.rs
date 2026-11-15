use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

#[test]
fn refuses_to_overwrite_input() {
    // Concurrent test runs share CARGO_TARGET_TMPDIR, so each process gets its own directory.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("refuses_to_overwrite_input-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.c");
    let source = "int main(void) { return 0; }\n";
    std::fs::write(&input, source).unwrap();
    let alias = dir.join("alias.c");
    symlink(&input, &alias).unwrap();

    for output in [&input, &alias] {
        let status = Command::new(env!("CARGO_BIN_EXE_transpiler"))
            .arg(&input)
            .arg("-o")
            .arg(output)
            .status()
            .unwrap();
        assert!(!status.success());
    }

    assert_eq!(std::fs::read_to_string(&input).unwrap(), source);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `--list` goes on after a refusal: each function's refusals are listed,
/// where the translation names the first only.
#[test]
fn lists_the_refusals_of_every_function() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("lists_the_refusals-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.c");
    let source = "#include <stdio.h>\n\
        static int first(int n) {\nagain:\n    n++;\n    if (n < 2) goto again;\n    return n;\n}\n\
        static int second(int n, ...) {\n    return n;\n}\n\
        int main(void) {\n    int n = 0;\nstart:\n    printf(\"%d\\n\", first(n));\n    \
        if (n++ < 1) goto start;\n    return second(0);\n}\n";
    std::fs::write(&input, source).unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_transpiler"))
            .args(args)
            .arg(&input)
            .output()
            .unwrap();
        assert!(!output.status.success());
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    let (_, refusal) = run(&[]);
    let (listed, _) = run(&["--list"]);
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(
        lines,
        [
            "rules\tfirst\tunsupported: `goto again`: no rule yet\t2",
            "rules\tsecond\tunsupported: the variadic function `second`\t1",
            "rules\tmain\tunsupported: `goto start`: no rule yet\t2",
        ]
    );
    assert_eq!(refusal.trim_end(), "unsupported: `goto again`: no rule yet");
    std::fs::remove_dir_all(&dir).unwrap();
}
