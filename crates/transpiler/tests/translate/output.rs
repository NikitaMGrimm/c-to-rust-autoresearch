use crate::common::{assert_lines, check, translate};

#[test]
fn fprintf_fputs_and_fputc_print_to_stdout_and_stderr() {
    let rust = check(
        "streams",
        r#"
#include <stdio.h>
#include <stdlib.h>
void say(const char *text) {
    fputs(text, stdout);
    fputs(text, stderr);
}
int main(int argc, char *argv[]) {
    if (argc != 2) {
        fprintf(stderr, "Usage: %s n\n", argv[0]);
        return 1;
    }
    int n = atoi(argv[1]);
    char word[8] = "bytes";
    for (int i = 0; i < n; i++) {
        fprintf(stdout, "%3d|%-4s|%x {}\n", i, "ab", i * 30);
        fprintf(stderr, "%.2f", i / 3.0);
    }
    fprintf(stdout, "no newline");
    fputs(" {literal}\n", stdout);
    fputs(word + 2, stdout);
    say("said");
    fputc('A' + n, stdout);
    putc(word[0], stdout);
    fputc('\n', stdout);
    putc('e', stderr);
    return 0;
}
"#,
        &[&[], &["0"], &["3"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "eprintln!(\"Usage: {} n\", args[0]);",
            "println!(\"{:3}|{:<4}|{:x} {{}}\", i, \"ab\", (i * 30) as u32);",
            "eprint!(\"{:.2}\", i as f64 / 3.0);",
            "print!(\"no newline\");",
            "println!(\" {{literal}}\");",
            "print!(\"{}\", text_of(&word[2..]));",
            "fn say(text: &str) {",
            "print!(\"{}\", text);",
            "eprint!(\"{}\", text);",
            "print!(\"{}\", (65 + n) as u8 as char);",
            "print!(\"{}\", word[0] as char);",
            "eprint!(\"{}\", 101 as u8 as char);",
        ],
    );
}

#[test]
fn output_to_another_file_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "fputs-file",
            "#include <stdio.h>\nint main(void) {\n    fputs(\"x\", fopen(\"/dev/null\", \"w\"));\n    return 0;\n}\n"
        ),
        "unsupported: `fputs` on a file other than `stdout` or `stderr`: no rule yet, at line 3"
    );
}

#[test]
fn sprintf_and_snprintf_store_the_text_with_a_nul() {
    let rust = check(
        "store",
        r#"
#include <stdio.h>
#include <stdlib.h>
struct entry { char name[12]; };
int name(char *out, size_t size, int id) { return snprintf(out, size, "item-%d", id); }
void fill(char *out, int id) { sprintf(out, "Entry_%d", id); }
int main(int argc, char *argv[]) {
    if (argc != 2) return 2;
    int size = atoi(argv[1]);
    char buffer[16];
    struct entry e;
    fill(buffer, size);
    int n = sprintf(e.name, "%s:%x {}", argv[1], size);
    printf("%d %s %s\n", n, buffer, e.name);
    char wide[11] = "abcdefghij";
    sprintf(wide + 2, "%c%%%5.1f", 'A', size / 4.0);
    printf("%s\n", wide);
    n = name(buffer, size, 42);
    printf("%d %s\n", n, size ? buffer : "");
    n = snprintf(buffer, sizeof buffer, "%s and more than sixteen", "text");
    printf("%d %s\n", n, buffer);
    if (snprintf(buffer, 4, "%d", size) >= 4) printf("cut %s\n", buffer);
    snprintf(buffer, 0, "none");
    printf("%s\n", buffer);
    return 0;
}
"#,
        &[&["0"], &["1"], &["7"], &["8"], &["999"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "store_text(&format!(\"Entry_{}\", id), out);",
            "store_text_within(&format!(\"item-{}\", id), out, size)",
            "&format!(\"{}:{:x} {{}}\", args[1], size as u32),",
            "&mut e.name,",
            "&format!(\"{}%{:5.1}\", 65 as u8 as char, size as f64 / 4.0),",
            "&mut wide[2..],",
            "if store_text_cut(&format!(\"{}\", size), &mut buffer[..4]) >= 4 {",
            "store_text_cut(\"none\", &mut buffer[..0]);",
            "fn store_text(text: &str, target: &mut [u8]) -> i32 {",
        ],
    );
}

#[test]
fn text_that_would_store_other_bytes_is_refused() {
    let refused = |name, c| translate(name, c).unwrap_err();
    assert_eq!(
        refused(
            "sprintf-bytes",
            "#include <stdio.h>\nint main(void) {\n    char source[8] = \"abc\", target[16];\n    sprintf(target, \"<%s>\", source);\n    return target[0];\n}\n"
        ),
        "unsupported: `%s` of bytes into bytes: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "sprintf-char",
            "#include <stdio.h>\nint main(int argc, char **argv) {\n    char target[4];\n    sprintf(target, \"%c\", argc + 'a');\n    return target[0];\n}\n"
        ),
        "unsupported: `%c` of a value that may be above 127 into bytes: no rule yet, at line 4"
    );
    assert_eq!(
        refused(
            "snprintf-beyond",
            "#include <stdio.h>\nint main(void) {\n    char target[8];\n    snprintf(target, 16, \"%d\", 7);\n    return target[0];\n}\n"
        ),
        "unsupported: an `snprintf` size beyond its buffer, at line 4"
    );
}
