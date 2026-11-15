use std::path::{Path, PathBuf};

/// The public development programs.
fn dev_programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/dev");
    let mut programs: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .collect();
    programs.sort();
    programs
}

/// Every stage ends with a translation or with what it has no rule for,
/// never with a panic. Run with `--nocapture` to see each outcome.
#[test]
fn every_development_program_is_translated_or_refused() {
    for path in dev_programs() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        match transpiler::translate_file(&path) {
            Ok(rust) => eprintln!("{name}: translated, {} lines", rust.lines().count()),
            Err(err) => {
                assert!(err.starts_with("unsupported: "), "{name}: {err}");
                eprintln!("{name}: {err}");
            }
        }
    }
}

/// A listing starts with the refusal that stops the translation, and is
/// empty for a program that is translated.
#[test]
fn a_listing_starts_with_the_refusal_of_the_translation() {
    for path in dev_programs() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let ast = transpiler::clang_ast(&path).unwrap();
        let refusal = transpiler::translate(&ast).err();
        let listed = transpiler::blockers(&ast);
        let first = listed.first().map(|blocker| blocker.refusal.clone());
        assert_eq!(first, refusal, "{name}");
        assert!(refusal.is_some() || listed.is_empty(), "{name}");
        for line in transpiler::blockers_file(&path).unwrap() {
            eprintln!("{name}\t{line}");
        }
    }
}
