use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

#[test]
fn refuses_to_overwrite_input() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("refuses_to_overwrite_input");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.c");
    let source = "int main(void) { return 0; }\n";
    std::fs::write(&input, source).unwrap();
    let alias = dir.join("alias.c");
    let _ = std::fs::remove_file(&alias);
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
}
