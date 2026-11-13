use crate::common::{assert_lines, check, translate};

#[test]
fn strcpy_strcat_and_strncat_copy_text_with_its_nul() {
    let rust = check(
        "strcpy",
        r#"
#include <stdio.h>
#include <string.h>
struct person { int age; char name[12]; };
void greet(char *to, const char *name) {
    strcpy(to, "hi ");
    strcat(to, name);
}
int main(int argc, char *argv[]) {
    struct person person;
    char line[32];
    char word[6] = "cdefg";
    char letters[3] = {'x', 'y', 'z'};
    strcpy(person.name, "Ada");
    greet(line, person.name);
    printf("%s|\n", line);
    strcpy(line + 3, &word[argc]);
    strcat(line, "");
    printf("%s|\n", line);
    strncat(line, word, argc);
    strncat(line, letters, 3);
    strncat(line, "\xc3\xa9", 0);
    printf("%s|%zu\n", line, strlen(line));
    return 0;
}
"#,
        &[&[], &["a"], &["a", "b", "c"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "copy_string(&mut *to, b\"hi \\0\");",
            "append_string(&mut *to, name);",
            "copy_string(&mut person.name, b\"Ada\\0\");",
            "copy_string(&mut line[3..], &word[args.len() as i32 as usize..]);",
            "append_string(&mut line, b\"\\0\");",
            "append_at_most(&mut line, &word, args.len() as i32 as usize);",
            "append_at_most(&mut line, &letters, 3);",
        ],
    );
}

#[test]
fn the_pointer_that_strcpy_returns_is_refused() {
    assert_eq!(
        translate(
            "strcpy-result",
            "#include <stdio.h>\n#include <string.h>\nint main(void) {\n    char text[8];\n    puts(strcpy(text, \"value\"));\n    return 0;\n}\n"
        )
        .unwrap_err(),
        "unsupported: a pointer from the library's `strcpy`, at line 5"
    );
}
