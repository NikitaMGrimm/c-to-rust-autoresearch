use crate::common::{assert_lines, check, translate};

/// R40: a search whose result is only tested, of text and of bytes, with
/// the NUL as the character, a count of 0 and an empty needle.
#[test]
fn a_search_that_is_only_tested_is_whether_it_found() {
    let rust = check(
        "search_tested",
        r#"
#include <stdio.h>
#include <string.h>
int next(int *k) { *k = *k + 1; return *k; }
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    char buf[8];
    snprintf(buf, sizeof buf, "%s", argv[1]);
    char tail[8] = "ab\0,;";
    char empty[1] = "";
    int k = 43, zero = 0;
    if (strchr(argv[1], ',')) puts("comma");
    printf("%d %d %d\n", strrchr(argv[1], 'a') == NULL, strchr(argv[1], 0) != NULL, !strchr(argv[1], 300));
    printf("%d %d %d\n", strchr(buf, ';') != NULL, strchr(tail, ',') != NULL, strchr(buf, zero) != NULL);
    printf("%d %d\n", strchr(argv[1], next(&k)) != NULL, k);
    printf("%d %d\n", memchr(buf, ',', 0) != NULL, memchr(tail, ';', 6) != NULL);
    printf("%d %d %d\n", strstr(argv[1], "b,") != NULL, strstr(buf, empty) != NULL, strstr(buf, "c") != NULL);
    printf("%d %d\n", strpbrk(argv[1], ",;") != NULL, strpbrk(buf, "") != NULL);
    return 0;
}
"#,
        &[&["a,b"], &[",ab"], &["ab;"], &["abc"], &[""]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "if args[1].contains(',') {",
            "i32::from(find_byte(args[1].as_bytes(), b'\\0').is_some()),",
            "i32::from(!args[1].contains(',')),",
            "i32::from(find_byte(until_nul(&buf), zero as u8).is_some()),",
            "i32::from(buf.iter().take(0).position(|&found| found == b',').is_some()),",
            "i32::from(args[1].contains(\"b,\")),",
            "i32::from(find_bytes(until_nul(&buf), until_nul(&empty)).is_some()),",
            "i32::from(find_any(args[1].as_bytes(), b\",;\").is_some()),",
        ],
    );
}

/// R40: a local that only holds what searches found and is only tested is
/// an `Option<usize>`.
#[test]
fn a_local_that_a_search_found_and_that_is_only_tested_is_an_option() {
    let rust = check(
        "search_local",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int check(const char *text) {
    char *found = strchr(text, ',');
    if (found != NULL) return 1;
    found = strstr(text, ";;");
    if (found) return 2;
    found = NULL;
    if (strlen(text) > 1) found = strrchr(text, 'c');
    return !found ? 3 : 4;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    char *path = getenv("PATH");
    printf("%d %d %d\n", check(argv[1]), check("x;c"), path != NULL && strchr(path, '/') != NULL);
    return 0;
}
"#,
        &[&["a,b"], &["a;;"], &["xc"], &[""]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut found: Option<usize> = text.find(',');",
            "if found.is_some() {",
            "found = text.find(\";;\");",
            "found = None;",
        ],
    );
}

/// What the search found is refused where it is used as a pointer.
#[test]
fn a_search_whose_place_is_used_is_refused() {
    let refused = |name, body: &str| {
        let c = format!(
            "#include <stdio.h>\n#include <string.h>\nint main(int argc, char *argv[]) {{\n    char buf[8] = \"a,b\";\n{body}\n    return 0;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    assert_eq!(
        refused("search_stored", "    *strchr(buf, ',') = ';';"),
        "unsupported: a pointer from the library's `strchr`, at line 5"
    );
    assert_eq!(
        refused(
            "search_effect",
            "    int k = 43;\n    printf(\"%d\\n\", memchr(buf, k++, 3) != NULL);"
        ),
        "unsupported: `memchr` of a byte with a side effect: no rule yet, at line 6"
    );
    assert_eq!(
        refused(
            "search_ints",
            "    int ints[2] = {1, 2};\n    printf(\"%d\\n\", memchr(ints, 1, sizeof ints) != NULL);"
        ),
        "unsupported: `memchr` of other than bytes: no rule yet, at line 6"
    );
    assert_eq!(
        refused(
            "search_text",
            "    printf(\"%d\\n\", memchr(argv[0], 'a', 1) != NULL);"
        ),
        "unsupported: `memchr` of text: no rule yet, at line 5"
    );
}

/// A local that keeps what a search found and uses it as a pointer, or a
/// search that goes on from it, has no rule yet.
#[test]
fn a_place_that_a_search_found_and_that_is_used_is_refused() {
    let refused = |name, body: &str| {
        let c = format!(
            "#include <stdio.h>\n#include <string.h>\nint main(void) {{\n    char buf[8] = \"a,b,c\";\n{body}\n    return 0;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    let used = "unsupported: the pointer `p`: a place that a search found, used as a pointer: no rule yet, at line 5";
    assert_eq!(
        refused(
            "search_cut",
            "    char *p = strchr(buf, ',');\n    if (p) *p = 0;"
        ),
        used
    );
    assert_eq!(
        refused(
            "search_again",
            "    char *p = strchr(buf, ',');\n    while (p) p = strchr(p + 1, ',');"
        ),
        used
    );
    assert_eq!(
        refused(
            "search_offset",
            "    char *p = strrchr(buf, ',');\n    if (p) printf(\"%ld\\n\", p - buf);"
        ),
        used
    );
}

/// A local pointer that no search reaches is no search's result.
#[test]
fn a_pointer_that_only_holds_null_stays_a_pointer() {
    let rust = check(
        "search_null",
        "#include <stdio.h>\nint main(void) {\n    char *q = NULL;\n    if (!q) puts(\"null\");\n    return 0;\n}\n",
        &[&[]],
        "",
    );
    assert_lines(&rust, &["let q: Option<&str> = None;"]);
}
