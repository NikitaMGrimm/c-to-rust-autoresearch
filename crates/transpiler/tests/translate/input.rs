use crate::common::{assert_lines, check, translate};

const NUMBERS: &str = r#"
#include <stdio.h>
int main(void) {
    int data[8];
    int count = 0;
    while (count < 8) {
        if (scanf("%d", &data[count]) != 1) {
            break;
        }
        count++;
    }
    unsigned total = 0;
    short small = 7;
    long wide = -1;
    char letter = '?', next = '!';
    scanf(" %c%c", &letter, &next);
    if (scanf("%u %hd", &total, &small) == 2) {
        printf("both\n");
    } else {
        printf("not both\n");
    }
    if (fscanf(stdin, "%ld", &wide) < 1)
        printf("no long\n");
    else
        printf("a long\n");
    for (int i = 0; i < count; i++) {
        printf("%d ", data[i]);
    }
    printf("\n%c%c %u %d %ld\n", letter, next, total, small, wide);
    return count;
}
"#;

#[test]
fn scanf_reads_what_c_reads() {
    let mut rust = String::new();
    for stdin in [
        "1 2 3 x y 4000000000 -5 123456789012",
        "",
        "  -7\n+8 9\n\n",
        "1 2 3 4 5 6 7 8 9 10 11 12",
        "12abc 5",
        "- 3",
        "3 q\n\n z 5 x",
        "3 ab 5 6x",
        "99999999999999999999999 zz 1 2 -99999999999999999999999",
    ] {
        rust = check("scanf", NUMBERS, &[&[]], stdin);
    }
    assert_lines(
        &rust,
        &[
            "let Some(scanned) = scan_int() else {",
            "data[count as usize] = scanned as i32;",
            "if let Some(scanned) = read_byte() {",
            "letter = scanned;",
            "small = scanned as i16;",
            "if let Some(scanned) = scan_int() {",
            "wide = scanned;",
            "use std::io::BufRead;",
        ],
    );
}

#[test]
fn scanf_with_no_rule_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "scanf-eof",
            "#include <stdio.h>\nint main(void) {\n    int x;\n    return scanf(\"%d\", &x) == EOF;\n}\n"
        ),
        "unsupported: the result of `scanf` other than compared with its number of conversions, at line 4"
    );
    assert_eq!(
        refused(
            "scanf-size",
            "#include <stdio.h>\nint main(void) {\n    long x;\n    scanf(\"%d\", &x);\n    return 0;\n}\n"
        ),
        "unsupported: a `scanf` conversion for a variable of another size, at line 4"
    );
    assert_eq!(
        refused(
            "scanf-text",
            "#include <stdio.h>\nint main(void) {\n    int x;\n    scanf(\"x=%d\", &x);\n    return 0;\n}\n"
        ),
        "unsupported: text in a `scanf` format: no rule yet, at line 4"
    );
}
