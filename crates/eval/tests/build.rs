use std::path::Path;
use std::process::Command;

#[test]
fn plain_cargo_commands_leave_the_transpiler_out() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(eval::REPO)
        .output()
        .unwrap();
    assert!(output.status.success());
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let defaults = metadata["workspace_default_members"].as_array().unwrap();
    assert!(!defaults.is_empty());
    assert!(
        defaults
            .iter()
            .all(|id| !id.as_str().unwrap().contains("/crates/transpiler#")),
        "{defaults:?}"
    );
}

const LOCKFILE: &str = "version = 4\n\n[[package]]\nname = \"transpiler\"\nversion = \"0.1.0\"\n";

#[test]
fn the_transpiler_of_a_checkout_is_built_in_the_sandbox_from_its_unignored_files() {
    // Concurrent test runs share CARGO_TARGET_TMPDIR, so each process gets its own directory.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("build-sandbox-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let checkout = dir.join("checkout");
    let crate_dir = checkout.join("crates/transpiler");
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::create_dir_all(checkout.join("local")).unwrap();
    let escaped = dir.join("escaped");
    let files = [
        (".gitignore", "/local/\n".to_owned()),
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/transpiler\"]\n".to_owned(),
        ),
        ("Cargo.lock", LOCKFILE.to_owned()),
        (
            "crates/transpiler/Cargo.toml",
            "[package]\nname = \"transpiler\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
                .to_owned(),
        ),
        ("crates/transpiler/src/main.rs", "fn main() {}\n".to_owned()),
        (
            "crates/transpiler/build.rs",
            format!("fn main() {{\n    let _ = std::fs::write({escaped:?}, \"\");\n}}\n"),
        ),
        ("local/secret", "private\n".to_owned()),
    ];
    for (path, contents) in files {
        std::fs::write(checkout.join(path), contents).unwrap();
    }
    std::os::unix::fs::symlink(checkout.join("local/secret"), crate_dir.join("link")).unwrap();
    for args in [&["init", "-q"][..], &["add", "-A"]] {
        let status = Command::new("git")
            .args(args)
            .current_dir(&checkout)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let workspace = dir.join("workspace");
    std::fs::create_dir(&workspace).unwrap();

    let built = eval::build_transpiler(&checkout, &workspace);

    let copied_ignored = workspace.join("local").exists();
    let link_kept = workspace
        .join("crates/transpiler/link")
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_symlink());
    let escaped = escaped.exists();
    std::fs::remove_dir_all(&dir).unwrap();
    let transpiler = built.unwrap();
    assert!(transpiler.starts_with(&workspace));
    assert!(!copied_ignored);
    assert!(link_kept);
    assert!(!escaped);
}
