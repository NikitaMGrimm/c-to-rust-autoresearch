use std::path::Path;

#[test]
fn reads_clang_ast() {
    let input = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reads_clang_ast.c");
    std::fs::write(&input, "int main(void) { return 0; }\n").unwrap();

    let ast = transpiler::clang_ast(&input).unwrap();

    assert_eq!(ast["kind"], "TranslationUnitDecl");
    assert!(
        ast["inner"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decl| decl["kind"] == "FunctionDecl" && decl["name"] == "main")
    );
}

#[test]
fn reports_invalid_c() {
    let input = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reports_invalid_c.c");
    std::fs::write(&input, "int main(void) { return }\n").unwrap();

    let err = transpiler::clang_ast(&input).unwrap_err();

    assert!(err.contains("expected expression"), "{err}");
}
