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

#[test]
fn a_function_that_returns_what_a_search_found_is_a_search() {
    let rust = check(
        "found_returned",
        r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
char *find(const char *buffer, size_t size, char target) {
    if (!buffer) return NULL;
    return (char *)memchr(buffer, target, size);
}
char *find_text(const char *text, char target) {
    return find(text, strlen(text), target);
}
char *copy(const char *text) {
    char *buffer = malloc(strlen(text) + 1);
    if (buffer) strcpy(buffer, text);
    return buffer;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    char c = (char)atoi(argv[1]);
    char fixed[9] = "abcdefgh";
    char *at = find(fixed, 4, c);
    if (at) {
        *at = '_';
        printf("%d %s\n", (int)(at - fixed), fixed);
    }
    char *in_text = find_text(fixed, c);
    if (in_text) puts(in_text);
    char *buffer = copy("look for the p in here");
    char *found = find(buffer, strlen(buffer), c);
    if (found) {
        printf("at %d: %c\n", (int)(found - buffer), *found);
    } else {
        puts("none");
    }
    free(buffer);
    buffer = copy("then a p here");
    found = find(buffer, strlen(buffer), c);
    if (found) printf("then at %d\n", (int)(found - buffer));
    free(buffer);
    return 0;
}
"#,
        &[&["112"], &["122"], &["97"], &["101"], &["104"], &["0"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "fn find(buffer: &[u8], size: u64, target: u8) -> Option<usize> {",
            "buffer.iter().take(size as usize).position(|&found| found == target)",
            "find(text, until_nul(text).len() as u64, target)",
            "let at = find(&fixed, 4, c);",
            "fixed[at] = b'_';",
            "println!(\"{}\", text_of(&fixed[in_text..]));",
            "found = find(&buffer, until_nul(&buffer).len() as u64, c);",
            "println!(\"then at {}\", found as i64 as i32);",
            "if let Some(found) = found {",
        ],
    );
    let refused = |name: &str, function: &str, body: &str| {
        let c = format!(
            "#include <stdio.h>\n#include <string.h>\n{function}\nint main(int argc, char *argv[]) {{\n    char one[4] = \"a,b\";\n    char two[4] = \"c,d\";\n{body}\n    return argc;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    let find = "char *find(const char *s, int c) { return strchr(s, c); }";
    for (name, function, body) in [
        // The result is used as a pointer where no test holds.
        (
            "returned_untested",
            find,
            "    char *p = find(one, ',');\n    puts(p ? p : two);",
        ),
        (
            "returned_in_test",
            find,
            "    if (find(one, ',')) puts(two);",
        ),
        (
            "returned_kept",
            find,
            "    char *(*search)(const char *, int) = find;\n    char *p = search(one, ',');\n    if (p) puts(two);",
        ),
        // The array is another one where the place is used.
        (
            "returned_base_stored",
            find,
            "    char *s = one;\n    s = one;\n    char *p = find(s, ',');\n    s = two;\n    if (p) puts(p);",
        ),
        (
            "returned_base_stored_under",
            find,
            "    char *s = one;\n    s = one;\n    char *p = find(s, ',');\n    if (p) { s = two; puts(p); }\n    puts(s);",
        ),
        // The place does not count from the argument.
        (
            "returned_offset",
            "char *find(const char *s, int c) { return strchr(s + 1, c); }",
            "    char *p = find(one, ',');\n    if (p) puts(p);",
        ),
        (
            "returned_moved",
            "char *find(const char *s, int c) { s++; return strchr(s, c); }",
            "    char *p = find(one, ',');\n    if (p) puts(p);",
        ),
        (
            "returned_second",
            "char *find(int c, const char *s) { return strchr(s, c); }",
            "    char *p = find(',', one);\n    if (p) puts(p);",
        ),
        (
            "returned_other",
            "char *find(char *s, int c) { if (c == 0) return s; return strchr(s, c); }",
            "    char *p = find(one, ',');\n    if (p) puts(p);",
        ),
    ] {
        assert_eq!(
            refused(name, function, body),
            "unsupported: the result of `find`: a place that a search found, used as a pointer: no rule yet",
            "{name}"
        );
    }
}

#[test]
fn a_loop_that_searches_on_from_what_it_found_counts_an_index() {
    let rust = check(
        "search_onward",
        r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static int count(const char *in, char c) {
    int res = 0;
    for (const char *s = in; s = strchr(s, c); s++) {
        res++;
    }
    return res;
}
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    char c = (char)atoi(argv[1]);
    char in[32] = "";
    strcpy(in, "abracadabra, a banana");
    printf("%d\n", count(in, c));
    const char *from = in;
    while ((from = strchr(from + 1, c)) != NULL) {
        printf("%d ", (int)(from - in));
    }
    char part[3] = "a";
    part[0] = c;
    for (const char *s = in; (s = strstr(s, part)) != NULL; s += strlen(part)) {
        printf("[%s]", s);
    }
    char *s = in;
    while ((s = strchr(s, c))) {
        if (s[1] == 'd') break;
        *s = '_';
        s += 1;
    }
    puts(in);
    return 0;
}
"#,
        &[&["97"], &["98"], &["110"], &["122"], &["44"], &["32"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "let mut s: usize = 0;",
            "while let Some(at) = find_byte(until_nul(&in_[s..]), c) {",
            "s += at;",
            "s += 1;",
            "while let Some(at) = find_byte(until_nul(&in_[from + 1..]), c) {",
            "while let Some(at) = find_bytes(until_nul(&in_[s..]), until_nul(&part)) {",
            "s += until_nul(&part).len();",
            "from += 1 + at;",
            "print!(\"{} \", from as i64 as i32);",
            "in_[s] = b'_';",
        ],
    );
    let refused = |name: &str, body: &str| {
        let c = format!(
            "#include <stdio.h>\n#include <string.h>\nint main(int argc, char *argv[]) {{\n    char buf[8] = \"a,b,c\";\n    int n = 0;\n{body}\n    return n + argc;\n}}\n"
        );
        translate(name, &c).unwrap_err()
    };
    for (name, body) in [
        // NULL is read after the loop.
        (
            "onward_after",
            "    char *p = buf;\n    while ((p = strchr(p, ','))) p++;\n    n = p == NULL;",
        ),
        // The value of a step is read.
        (
            "onward_value",
            "    for (char *p = buf; (p = strchr(p, ','));) *p++ = '.';",
        ),
        // Another pointer is stored in it.
        (
            "onward_stored",
            "    for (char *p = buf; (p = strchr(p, ',')); p++) if (n++ == 0) p = buf + 3;",
        ),
        // It does not start at the array.
        (
            "onward_offset",
            "    for (char *p = buf + 1; (p = strchr(p, ',')); p++) n++;",
        ),
        // The search reads the pointer twice.
        (
            "onward_length",
            "    for (char *p = buf; (p = memchr(p, ',', 5 - (p - buf))); p++) n++;",
        ),
    ] {
        assert_eq!(
            refused(name, body),
            "unsupported: the pointer `p`: a place that a search found, used as a pointer: no rule yet, at line 6",
            "{name}"
        );
    }
}
