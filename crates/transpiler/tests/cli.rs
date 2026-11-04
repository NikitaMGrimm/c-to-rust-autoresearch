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
