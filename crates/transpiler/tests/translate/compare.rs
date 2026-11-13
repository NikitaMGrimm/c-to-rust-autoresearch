use crate::common::{assert_lines, check, translate};

#[test]
fn strcmp_tested_against_zero_compares_the_texts() {
    let rust = check(
        "strcmp",
        r#"
#include <stdio.h>
#include <string.h>
struct user { char name[4]; int age; };
static int before(const char *a, const char *b) { return strcmp(a, b) < 0; }
int main(int argc, char *argv[]) {
    if (argc < 3) return 2;
    const char *mode = argc > 3 ? "fast" : "slow";
    char word[6] = "fast";
    char high[3] = "\xe9";
    struct user users[2] = {{"ann", 3}, {"bob", 4}};
    if (strcmp(argv[1], "on") == 0) printf("on\n");
    else if (!strcmp(argv[1], "off")) printf("off\n");
    printf("%d %d\n", strcmp(argv[1], argv[2]) < 0, 0 <= strcmp(argv[1], argv[2]));
    printf("%d %d\n", strcmp(mode, "fast") != 0, strcmp(word, mode) == 0);
    printf("%d %d\n", strcmp(word, "fat") > 0, strcmp(argv[2], word) >= 0);
    printf("%d %d\n", strcmp(high, word) > 0, strcmp(word + 2, "st") == 0);
    printf("%d %d\n", strcmp(argv[1], "") == 0, strcmp(users[1].name, argv[1]) == 0);
    printf("%d %d\n", before(users[0].name, users[1].name), before(word, high));
    int i = 0;
    while (i < 2 && strcmp(users[i].name, "bob")) i++;
    printf("%d %d\n", i, strcmp(word, word) == 0);
    return 0;
}
"#,
        &[
            &["on", "off"],
            &["off", "off"],
            &["", "a", "x"],
            &["bob", "fast"],
            &["fat", "\u{e9}"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "i32::from(until_nul(a) < until_nul(b))",
            "if args[1] == \"on\" {",
            "} else if args[1] == \"off\" {",
            "i32::from(args[1].as_str() < args[2].as_str()),",
            "i32::from(args[1].as_str() >= args[2].as_str()),",
            "i32::from(mode != \"fast\"),",
            "i32::from(until_nul(&word) == mode.as_bytes()),",
            "i32::from(until_nul(&word) > b\"fat\".as_slice()),",
            "i32::from(args[2].as_bytes() >= until_nul(&word)),",
            "i32::from(until_nul(&word[2..]) == b\"st\"),",
            "i32::from(args[1].is_empty()),",
            "i32::from(until_nul(&users[1].name) == args[1].as_bytes()),",
            "while i < 2 && until_nul(&users[i as usize].name) != b\"bob\" {",
            "println!(\"{} {}\", i, i32::from(true));",
        ],
    );
}

#[test]
fn strncmp_compares_within_the_count() {
    let rust = check(
        "strncmp",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char *argv[]) {
    if (argc < 4) return 2;
    int n = atoi(argv[3]);
    if (n < 0) return 3;
    char full[4] = {'a', 'b', 'c', 'd'};
    char word[7] = "prefix";
    const char *text = "prefab";
    printf("%d %d\n", strncmp(argv[1], argv[2], n) == 0, strncmp(word, text, n) < 0);
    printf("%d %d\n", strncmp(full, "abcd", 4) == 0, strncmp(word, "pre", 3) == 0);
    printf("%d %d\n", strncmp(word, argv[1], 0) == 0, strncmp("pre", word, n) > 0);
    if (strncmp(argv[1], text, strlen(text)) == 0) printf("starts\n");
    return 0;
}
"#,
        &[
            &["prefix", "pre", "3"],
            &["prefab", "prefix", "6"],
            &["", "x", "100"],
            &["a", "b", "0"],
        ],
        "",
    );
    assert_lines(
        &rust,
        &[
            "if first_bytes(args[1].as_bytes(), text.len() as u64 as usize)",
            "== first_bytes(text.as_bytes(), text.len() as u64 as usize)",
            "text_within(&word, n as usize) < first_bytes(text.as_bytes(), n as usize),",
            "i32::from(text_within(&full, 4) == b\"abcd\"),",
            "i32::from(text_within(&word, 3) == b\"pre\"),",
            "i32::from(true),",
            "fn text_within(bytes: &[u8], count: usize) -> &[u8] {",
            "fn first_bytes(text: &[u8], count: usize) -> &[u8] {",
        ],
    );
}

#[test]
fn the_value_of_a_comparison_other_than_its_sign_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "strcmp-printed",
            "#include <stdio.h>\n#include <string.h>\nint main(void) {\n    printf(\"%d\\n\", strcmp(\"a\", \"b\"));\n    return 0;\n}\n"
        ),
        "unsupported: the value of `strcmp` other than its sign: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "strncmp-stored",
            "#include <string.h>\nint main(void) {\n    int r = strncmp(\"a\", \"b\", 1);\n    return r < 0;\n}\n"
        ),
        "unsupported: the value of `strncmp` other than its sign: no rule yet, at line 3"
    );
    assert_eq!(
        refused(
            "strcmp-added",
            "#include <string.h>\nint main(void) {\n    return strcmp(\"a\", \"b\") + 1 > 0;\n}\n"
        ),
        "unsupported: the value of `strcmp` other than its sign: no rule yet, at line 3"
    );
    assert_eq!(
        refused(
            "strncmp-count",
            "#include <string.h>\nint main(void) {\n    int n = 1;\n    return strncmp(\"ab\", \"ac\", n++) == 0;\n}\n"
        ),
        "unsupported: a count of `strncmp` that calls or assigns: no rule yet, at line 4"
    );
}

#[test]
fn memcmp_tested_against_zero_compares_the_elements() {
    let rust = check(
        "memcmp",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
struct pair { int left[3]; int right[3]; };
static int same(const short *x, const short *y, int n) {
    return memcmp(x, y, n * sizeof(short)) == 0;
}
int main(int argc, char *argv[]) {
    if (argc < 2) return 2;
    int n = atoi(argv[1]);
    if (n < 0 || n > 4) return 3;
    unsigned char a[4] = {1, 2, 200, 4};
    unsigned char b[4] = {1, 2, 3, 4};
    char word[5] = "help";
    short s[4] = {1, 2, 3, 4};
    short t[4] = {1, 2, 9, 4};
    struct pair p = {{7, 8, 9}, {7, 8, 0}};
    int x = 3, y = 3;
    printf("%d %d %d\n", memcmp(a, b, sizeof a) > 0, memcmp(a, b, n) < 0, !memcmp(a, b, 2));
    printf("%d %d\n", memcmp(word, "hel", 3) == 0, memcmp(word, "help", 5) == 0);
    printf("%d %d\n", same(s, t, n), memcmp(p.left, p.right, 2 * sizeof(int)) == 0);
    printf("%d %d\n", memcmp(&x, &y, sizeof x) == 0, memcmp(s, t, 0) == 0);
    if (memcmp(p.left, p.right, sizeof p.left)) printf("differ\n");
    return 0;
}
"#,
        &[&["0"], &["2"], &["3"], &["4"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "i32::from(x[..n as usize] == y[..n as usize])",
            "i32::from(a > b),",
            "i32::from(a[..n as usize] < b[..n as usize]),",
            "i32::from(a[..2] == b[..2]),",
            "i32::from(word[..3] == b\"hel\\0\"[..3]),",
            "i32::from(word[..5] == b\"help\\0\"[..5]),",
            "println!(\"{} {}\", same(&s, &t, n), i32::from(p.left[..2] == p.right[..2]));",
            "println!(\"{} {}\", i32::from(x == y), i32::from(true));",
            "if p.left != p.right {",
        ],
    );
}

#[test]
fn memcmp_whose_bytes_differ_from_the_values_is_refused() {
    let refused = |name: &str, c: &str| translate(name, c).unwrap_err();
    let program = |declarations: &str, size: &str, test: &str| {
        format!(
            "#include <string.h>\n{declarations}\nint main(void) {{\n    return memcmp(a, b, {size}) {test};\n}}\n"
        )
    };
    assert_eq!(
        refused(
            "memcmp-order",
            &program("int a[2] = {1, 256}, b[2] = {2, 1};", "sizeof a", "< 0")
        ),
        "unsupported: an ordering by `memcmp` of numbers wider than a byte: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "memcmp-float",
            &program("double a[1] = {0.0}, b[1] = {-0.0};", "sizeof a", "== 0")
        ),
        "unsupported: `memcmp` of other than integers: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "memcmp-record",
            &program(
                "struct s { char c; int i; } a[1] = {{1, 2}}, b[1] = {{1, 2}};",
                "sizeof a",
                "== 0"
            )
        ),
        "unsupported: `memcmp` of other than integers: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "memcmp-part",
            &program("int a[2] = {1, 2}, b[2] = {1, 2};", "6", "== 0")
        ),
        "unsupported: a size of `memcmp` that is no whole number of elements, at line 4"
    );
    assert_eq!(
        refused(
            "memcmp-types",
            &program(
                "int a[2] = {1, 2}; unsigned b[2] = {1, 2};",
                "sizeof a",
                "== 0"
            )
        ),
        "unsupported: `memcmp` of two different types: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "memcmp-value",
            &program("char a[2] = \"a\", b[2] = \"b\";", "1", "+ 1 > 0")
        ),
        "unsupported: the value of `memcmp` other than its sign: no rule yet, at line 4"
    );
}
