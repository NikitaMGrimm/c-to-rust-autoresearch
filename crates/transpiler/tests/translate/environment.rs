use crate::common::{assert_lines, check, translate};

/// R39: `getenv` as `std::env::var`, whose value the slot owns. The tests
/// read a variable that no process has and `PATH`, which every one has,
/// and print nothing that differs between machines.
#[test]
fn a_variable_of_the_environment_is_an_owned_string() {
    let rust = check(
        "environment",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int number(const char *name, int otherwise) {
    char *v = getenv(name);
    if (!v) return otherwise;
    return atoi(v);
}
void show(const char *v) { printf("%d\n", v != NULL); }
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    printf("%d\n", number(argv[1], 42));
    char *path = getenv("PATH");
    if (path != NULL) {
        printf("%c %d %d\n", *path, strlen(path) > 0, strcmp(path, "") != 0);
    }
    const char *v = getenv("T3_TEST_UNSET");
    if (v == NULL) v = "7";
    printf("%d %d %d\n", atoi(v), getenv("PATH") != NULL, getenv("T3_TEST_UNSET") == NULL);
    show(getenv("T3_TEST_UNSET"));
    show(path);
    show(path);
    char name[5] = "PATH";
    printf("%d\n", getenv(name) != NULL && *getenv(name) == '/');
    return 0;
}
"#,
        &[&["T3_TEST_UNSET"], &["PATH"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let v: Option<String> = std::env::var(name).ok();",
            "let Some(v) = v.as_deref() else {",
            "let path: Option<String> = std::env::var(\"PATH\").ok();",
            "v = Some(String::from(\"7\"));",
            "show(std::env::var(\"T3_TEST_UNSET\").ok().as_deref());",
            "show(path.as_deref());",
            "i32::from(std::env::var(\"PATH\").is_ok()),",
            "i32::from(std::env::var(\"T3_TEST_UNSET\").is_err()),",
        ],
    );
}

/// The environment's text is not the program's to change, and a `String`
/// is walked by no cursor.
#[test]
fn a_variable_of_the_environment_that_is_changed_is_refused() {
    let refused = |name, body: &str| {
        let c = format!(
            "#include <stdio.h>\n#include <stdlib.h>\nint main(void) {{\n    char *v = getenv(\"PATH\");\n{body}\n    return 0;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    let changed = "unsupported: the pointer `v`: a variable of the environment that is stored through, moved, freed or read byte by byte: no rule yet, at line 4";
    assert_eq!(refused("stored", "    if (v) v[0] = 'x';"), changed);
    assert_eq!(refused("moved", "    if (v) v++;"), changed);
    assert_eq!(
        refused("set", "    setenv(\"A\", \"1\", 1);"),
        "unsupported: this call of the library's `setenv`: no rule yet, at line 5"
    );
    assert_eq!(
        translate(
            "kept",
            "#include <stdlib.h>\nconst char *path;\nint main(void) {\n    path = getenv(\"PATH\");\n    return path != NULL;\n}\n"
        )
        .unwrap_err(),
        "unsupported: the pointer `path`: a variable of the environment kept in a record or a global: no rule yet, at line 2"
    );
    // The `String` is the local's, and a borrow of it ends with the function.
    assert_eq!(
        translate(
            "borrowed",
            "#include <stdlib.h>\nconst char *get(void) {\n    const char *v = getenv(\"PATH\");\n    if (!v) return \"\";\n    return v;\n}\nint main(void) {\n    return *get() == 0;\n}\n"
        )
        .unwrap_err(),
        "unsupported: the result of `get`: it points to a variable that ends with the function"
    );
}
