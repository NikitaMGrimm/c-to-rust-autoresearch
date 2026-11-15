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
            "copy_string(to, b\"hi \\0\");",
            "append_string(to, name);",
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

#[test]
fn a_text_copy_within_one_array_is_refused() {
    // The two parts may not overlap in C, but Rust borrows the whole array
    // for each argument.
    assert_eq!(
        translate(
            "strcpy-within",
            "#include <stdio.h>\n#include <string.h>\nint main(void) {\n    char b[8] = \"abc\";\n    strcpy(b + 4, b);\n    printf(\"%s\\n\", b + 4);\n    return 0;\n}\n"
        )
        .unwrap_err(),
        "unsupported: `strcpy` within one array: no rule yet, at line 5"
    );
}

#[test]
fn memcpy_and_memmove_copy_slices_and_within_one_array() {
    let rust = check(
        "memcpy",
        r#"
#include <stdio.h>
#include <string.h>
struct state { int count; double level; char tag[4]; };
struct buffer { int length; char data[10]; };
void copy_ints(int *to, const int *from, int n) {
    memcpy(to, from, n * sizeof(int));
}
void shift(int *values, int length, int by) {
    memmove(values + by, values, (length - by) * sizeof(int));
}
void append(struct buffer *to, const struct buffer *from) {
    memcpy(to->data + to->length, from->data, from->length);
    to->length += from->length;
}
int main(int argc, char *argv[]) {
    int a[6] = {1, 2, 3, 4, 5, 6};
    int b[6] = {0};
    double x[3] = {1.5, 2.5, 3.5};
    double y[3];
    char s[12] = "0123456789";
    struct state now = {3, 0.5, "ab"};
    struct state backup = {0, 0.0, "z"};
    struct state list[2] = {{1, 1.0, "a"}, {2, 2.0, "b"}};
    struct buffer c;
    struct buffer d = {3, "xyz"};
    memcpy(b, a, sizeof(a));
    memcpy(b + 1, a + 3, 2 * sizeof(int));
    copy_ints(b + 4, a, argc);
    memcpy(y, x, sizeof(double) * 3);
    printf("%d ", backup.count);
    memcpy(&backup, &now, sizeof(struct state));
    memcpy(list, &backup, sizeof list[0]);
    memcpy(backup.tag, "cd", 3);
    memcpy(s, s, 0);
    memmove(s + 2, s, 5);
    memmove(s, s + 3, argc);
    memmove(&a[1], &a[0], 5 * sizeof(int));
    shift(b, 6, argc);
    c.length = 0;
    append(&c, &d);
    append(&c, &d);
    memcpy(&c.data[argc], &s[0], 1);
    c.data[c.length] = '\0';
    for (int i = 0; i < 6; i++) printf("%d %d ", a[i], b[i]);
    printf("%.1f %d %.1f %s %s %s %s\n", y[2], list[0].count, list[0].level, backup.tag, list[1].tag, s, c.data);
    return 0;
}
"#,
        &[&[], &["a"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "to[..n as usize].copy_from_slice(&from[..n as usize]);",
            "values.copy_within(..(length - by) as usize, by as usize);",
            "b.copy_from_slice(&a);",
            "b[1..][..2].copy_from_slice(&a[3..][..2]);",
            "y.copy_from_slice(&x);",
            "backup = now;",
            "list[0] = backup;",
            "backup.tag[..3].copy_from_slice(&b\"cd\\0\"[..3]);",
            "s.copy_within(..5, 2);",
            "s.copy_within(3..3 + args.len() as i32 as usize, 0);",
            "a.copy_within(..5, 1);",
            "c.data[args.len() as i32 as usize] = s[0];",
        ],
    );
}

#[test]
fn a_copy_that_is_no_count_of_elements_of_one_type_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    // Safe Rust does not read the bits of a `float` as an integer.
    assert_eq!(
        refused(
            "memcpy-types",
            "#include <string.h>\nint main(void) {\n    float f = 1.0f;\n    unsigned bits;\n    memcpy(&bits, &f, sizeof bits);\n    return bits == 0;\n}\n"
        ),
        "unsupported: a copy between two types, or of a type that is not copied, at line 5"
    );
    assert_eq!(
        refused(
            "memcpy-part",
            "#include <string.h>\nint main(void) {\n    int a[2] = {1, 2}, b[2];\n    memcpy(b, a, 6);\n    return b[0];\n}\n"
        ),
        "unsupported: a copy of part of an element, at line 4"
    );
    assert_eq!(
        refused(
            "memcpy-size",
            "#include <string.h>\nint main(int argc, char **argv) {\n    int a[2] = {1, 2}, b[2];\n    memcpy(b, a, argc);\n    return b[0];\n}\n"
        ),
        "unsupported: a copy whose size is no count of elements, at line 4"
    );
    assert_eq!(
        refused(
            "memmove-effects",
            "#include <string.h>\nint main(void) {\n    int a[4] = {1, 2, 3, 4}, i = 0;\n    memmove(a, a + ++i, 2 * sizeof(int));\n    return a[0];\n}\n"
        ),
        "unsupported: a copy from an offset with effects, at line 4"
    );
    // The size would be evaluated for the target and for the source.
    assert_eq!(
        refused(
            "memcpy-size-effects",
            "#include <string.h>\nint main(void) {\n    char a[4] = \"abc\", b[4];\n    int n = 0;\n    memcpy(b, a, n += 2);\n    return b[0] + n;\n}\n"
        ),
        "unsupported: a copy whose size calls or assigns: no rule yet, at line 5"
    );
}

/// R29: `sizeof` of a record's own type copies one record, also where it
/// has bit-fields.
#[test]
fn memcpy_of_a_record_with_bit_fields_is_an_assignment() {
    let rust = check(
        "copy_bit_fields",
        r#"
#include <stdio.h>
#include <string.h>
struct flags { unsigned on : 1; unsigned level : 3; };
struct state { struct flags flags; int value; };
int main(int argc, char *argv[]) {
    struct state now;
    struct state backup = {{0, 0}, 7};
    printf("%d\n", backup.value);
    now.flags.on = 1;
    now.flags.level = (unsigned)argc;
    now.value = 40 + argc;
    memcpy(&backup, &now, sizeof(struct state));
    now.value = 0;
    printf("%d %d %d %d\n", backup.flags.on, backup.flags.level, backup.value, now.value);
    return 0;
}
"#,
        &[&[], &["x", "y"]],
        "",
    );
    assert_lines(&rust, &["backup = now;"]);
}
