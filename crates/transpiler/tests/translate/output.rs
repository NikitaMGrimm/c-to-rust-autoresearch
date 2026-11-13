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
