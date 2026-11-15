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

/// A search that goes on from what it found has no rule yet.
#[test]
fn a_search_that_goes_on_from_its_result_is_refused() {
    let c = "#include <stdio.h>\n#include <string.h>\nint main(void) {\n    char buf[8] = \"a,b,c\";\n    char *p = strchr(buf, ',');\n    while (p) p = strchr(p + 1, ',');\n    return 0;\n}\n";
    assert_eq!(
        translate("search_again", c).unwrap_err(),
        "unsupported: the pointer `p`: a place that a search found, used as a pointer: no rule yet, at line 5"
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

/// R47: a place that a search found is an index into the array in the
/// branch where the test found it.
#[test]
fn a_found_place_is_an_index_where_its_test_holds() {
    let rust = check(
        "search_place",
        r#"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
static void mark(char *str, int c) {
    char *found = (char *)memchr(str, c, strlen(str));
    if (found) {
        *found = 'X';
    }
}
static const char *rest(const char *s, int c) {
    const char *at = strchr(s, c);
    if (at) return at + 1;
    return s;
}
int main(int argc, char *argv[]) {
    char buffer[40];
    if (argc != 2) return 2;
    int c = atoi(argv[1]);
    strcpy(buffer, "a text: with p in it: end");
    char *at = strchr(buffer, c);
    int where = -1;
    if (at != NULL) {
        where = (int)(at - buffer);
        printf("%ld %c %d %s\n", at - buffer, *at, at[1], at + 1);
        char *next = strchr(at + 1, c);
        if (next) puts("again");
    } else {
        puts("none");
    }
    char *last = strrchr(buffer, c);
    if (last == NULL) {
        puts("no last");
    } else {
        *last = '#';
        last[1] = '!';
    }
    mark(buffer, 't');
    printf("%d %s|%s\n", where, buffer, rest(buffer, ':'));
    return 0;
}
"#,
        &[&["112"], &["122"], &["58"], &["97"], &["100"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "if let Some(found) = found {",
            "str[found] = b'X';",
            "return &s[at + 1..];",
            "if let Some(at) = at {",
            "where_ = at as i64 as i32;",
            "if let Some(last) = last {",
            "buffer[last] = b'#';",
            "buffer[last + 1] = b'!';",
        ],
    );
    let refused = |name: &str, body: &str| {
        let c = format!(
            "#include <stdio.h>\n#include <string.h>\nint main(int argc, char *argv[]) {{\n    char buf[8] = \"a,b\";\n    char two[8] = \"c,d\";\n{body}\n    return argc;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    let place = |line: u32| {
        format!(
            "unsupported: the pointer `p`: a place that a search found, used as a pointer: no rule yet, at line {line}"
        )
    };
    for (name, body) in [
        (
            "place_outside",
            "    char *p = strchr(buf, ',');\n    if (p) puts(\"found\");\n    if (argc > 5) putchar(*p);",
        ),
        (
            "place_and",
            "    char *p = strchr(buf, ',');\n    if (p && p[1] == 'b') puts(\"found\");",
        ),
        (
            "place_moved",
            "    char *p = strchr(buf, ',');\n    if (p) { p++; puts(p); }",
        ),
        (
            "place_two",
            "    char *p = strchr(buf, ',');\n    if (!p) p = strchr(two, ',');\n    if (p) puts(p);",
        ),
        (
            "place_offset",
            "    char *p = strchr(buf + 1, ',');\n    if (p) printf(\"%ld\\n\", p - buf);",
        ),
    ] {
        assert_eq!(refused(name, body), place(6), "{name}");
    }
}
