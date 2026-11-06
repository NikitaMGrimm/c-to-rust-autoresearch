use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A fresh directory for one test, since concurrent runs share CARGO_TARGET_TMPDIR.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn translate(dir: &Path, c: &str) -> String {
    let input = dir.join("input.c");
    std::fs::write(&input, c).unwrap();
    transpiler::translate(&transpiler::clang_ast(&input).unwrap())
}

fn build(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run(program: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Checks that the translation of `c` prints and exits as the C program does
/// on each standard input, with overflow checks on as in the evaluator.
fn same_behaviour(test: &str, c: &str, stdins: &[&str]) -> String {
    let runs: Vec<(&[&str], &str)> = stdins.iter().map(|&stdin| (&[][..], stdin)).collect();
    same_behaviour_on(test, c, &runs)
}

/// `same_behaviour` on each pair of program arguments and standard input.
fn same_behaviour_on(test: &str, c: &str, runs: &[(&[&str], &str)]) -> String {
    let dir = scratch(test);
    let rust = translate(&dir, c);
    std::fs::write(dir.join("translation.rs"), &rust).unwrap();
    build(
        Command::new(transpiler::CLANG)
            .arg(dir.join("input.c"))
            .arg("-o")
            .arg(dir.join("c")),
    );
    build(
        Command::new("rustc")
            .args(["--edition", "2024", "-C", "overflow-checks=on", "-o"])
            .arg(dir.join("rust"))
            .arg(dir.join("translation.rs")),
    );
    for (args, stdin) in runs {
        let c = run(&dir.join("c"), args, stdin);
        let rust = run(&dir.join("rust"), args, stdin);
        assert_eq!(
            String::from_utf8_lossy(&rust.stdout),
            String::from_utf8_lossy(&c.stdout),
            "args {args:?}, stdin {stdin:?}"
        );
        assert_eq!(
            rust.status.code(),
            c.status.code(),
            "args {args:?}, stdin {stdin:?}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
    rust
}

#[test]
fn translates_int_functions_and_formatted_io() {
    let c = r#"
#include <stdio.h>
#include <stdlib.h>

int scale(int a, int b) {
    int c = a * 3 - b / 2;
    c %= 7;
    return -c + (a - b) * 2;
}

void show(int type) {
    type -= 1;
    printf("[%d] {%5d|%-4d|%+d|%03d} 100%%\n", type, type, type, type, type);
}

int main(void) {
    int x = 4, y;
    int n = scanf("%d %d", &x, &y);
    show(scale(x, n));
    printf("tab\there \"quoted\" \\ \101\x42 no newline");
    return n + 2;
}
"#;
    let rust = same_behaviour(
        "translates_int_functions_and_formatted_io",
        c,
        &[
            "17 -5",
            "  -3\n\t+12",
            "8",
            "",
            "x",
            "-",
            "-x 3",
            "99999999999999999999 1",
        ],
    );
    // glibc's inline helpers from stdlib.h are not part of the program.
    assert!(!rust.contains("bswap"), "{rust}");
    assert!(rust.contains("mut r#type: i32"), "{rust}");
}

#[test]
fn scanf_stores_into_a_repeated_destination_in_order() {
    let c = r#"
#include <stdio.h>

int main(void) {
    int x = 7, value = 0;
    int n = scanf("%d %d %d", &x, &value, &x);
    printf("%d %d %d\n", n, x, value);
    return x;
}
"#;
    same_behaviour(
        "scanf_stores_into_a_repeated_destination_in_order",
        c,
        &["1 2 3", "1 2 z", "1", "", "-4 5 6"],
    );
}

#[test]
fn formats_end_at_their_first_zero_byte() {
    let c = r#"
#include <stdio.h>

int main(void) {
    int x = 7;
    int n = scanf("%d\0%d", &x);
    printf("%d %d\n\0hidden %d\n", n, x);
    printf("a\0b");
    return x;
}
"#;
    same_behaviour(
        "formats_end_at_their_first_zero_byte",
        c,
        &["5\0 6", "5 6", "", "z"],
    );
}

#[test]
fn keeps_names_that_rust_reserves_apart() {
    // `self` and `main` need new Rust names, which must not be names the
    // program already uses.
    let c = r#"
#include <stdio.h>

int c_main(void) { return 3; }
int main_(void) { return 4; }

int main(void) {
    int self = 1;
    int self_ = 2;
    int Self = 5;
    int Self_ = 6;
    printf("%d %d %d %d %d %d\n", self, self_, Self, Self_, c_main(), main_());
    return self * 10 + Self;
}
"#;
    same_behaviour("keeps_names_that_rust_reserves_apart", c, &[""]);
}

#[test]
fn reports_unsupported_constructs_as_compile_errors() {
    let dir = scratch("reports_unsupported_constructs_as_compile_errors");
    let rust = translate(
        &dir,
        "#include <stdio.h>\nint main(void) { printf(\"%f\\n\", 1.5); return 0; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: printf conversion %f")"#),
        "{rust}"
    );
    // A block-scope extern refers to glibc's optind, which is 1, not 0.
    let rust = translate(
        &dir,
        "int main(void) { extern int optind; return optind; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: extern local variable")"#),
        "{rust}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_control_flow_and_goto() {
    let c = r#"
#include <stdio.h>

int seen(int v) {
    printf("seen %d\n", v);
    return v;
}

int loops(int n) {
    int sum = 0;
    for (int i = 0; i < n; i++) {
        if (i % 3 == 0)
            continue;
        for (int j = 0;; j++) {
            if (j >= i)
                break;
            sum += j;
        }
        if (sum > 40)
            break;
    }
    int k = n;
    do {
        k--;
        if (k % 2)
            continue;
        sum += k;
    } while (k > 0);
    while (1) {
        if (!(sum < 100) || n == 0)
            break;
        sum += n;
    }
    return sum;
}

int jumps(int x, int y) {
    unsigned char wrap = 250;
    while (x > 0 || y > 0) {
        printf("x %d y %d\n", x, y);
        if (x == 1 && y == 4) {
            goto skip;
        }
    again:
        if (x > 0) {
            x--;
        }
    skip:
        wrap++;
        if (y == 0) {
            continue;
        }
        y--;
        for (int i = 0; i < 3; i++) {
            if (i == x)
                goto again;
        }
    }
    if (x == 0) goto out;
    printf("unreachable\n");
out:
    return wrap;
}

int main(void) {
    int n = 0, m = 0;
    scanf("%d %d", &n, &m);
    int t = (n < m) + (n == m) * 2 - !n;
    if (n > 5) {
        printf("big\n");
    } else if (seen(n) && seen(m) || seen(-1)) {
        printf("both\n");
    } else {
        printf("neither\n");
    }
    printf("%d %d %d\n", t, loops(n), jumps(n, m));
    return 0;
}
"#;
    let rust = same_behaviour(
        "translates_control_flow_and_goto",
        c,
        &["0 0", "1 4", "2 5", "4 1", "7 3", "3 3", "0 9"],
    );
    assert!(rust.contains("__goto"), "{rust}");
}

#[test]
fn rejects_jumps_it_cannot_translate() {
    let dir = scratch("rejects_jumps_it_cannot_translate");
    let rust = translate(
        &dir,
        "int main(void) { int x = 1; goto in; if (x) { in: x = 2; } return x; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: goto into a block")"#),
        "{rust}"
    );
    // Declaring the inner x before the block would hide the outer one that
    // the block assigns first.
    let rust = translate(
        &dir,
        "int main(void) { int x = 1; { again: x = x + 1; int x = 5;\n\
         if (x < 0) goto again; } return x; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: declaration in a block with labels")"#),
        "{rust}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_characters_and_unsigned_conversions() {
    let c = r#"
#include <stdio.h>

int main(void) {
    char a = '-', b = 'z';
    int n = scanf("%c%c", &a, &b);
    char c = '?';
    int m = fscanf(stdin, " %c%%", &c);
    unsigned int big = 4000000000u;
    printf("%d %d %d|%d %d|\n", n, m, '\xff' + '\0', 'o', -'\n');
    printf("%x %X %o %u|%+u|%-6x|%08X|%3o\n", a, b, c, a - 'a', big, c, big, '\377');
    return c + 1;
}
"#;
    let rust = same_behaviour(
        "translates_characters_and_unsigned_conversions",
        c,
        &["ab c%", "a", "", " \n x", "\u{80}\u{ff}", "abc", "ab\tc"],
    );
    assert!(rust.contains("as u32"), "{rust}");
}

#[test]
fn translates_pointers_to_string_literals() {
    let c = r#"
#include <stdio.h>

void show(const char *line) {
    if (line != NULL && !(line == 0)) {
        printf("[%s] %s\n", line, "lit");
    } else if (!line) {
        printf("null %d\n", line == NULL);
    }
}

const char *pick(int n) {
    if (n > 2)
        return "big, 100%";
    if (n)
        return ("h\303\251llo {}");
    return (char *)0;
}

int main(void) {
    int n = 0;
    scanf("%d", &n);
    char *p;
    char *none = 0;
    p = "a\0b";
    show(p);
    show(none);
    show(pick(n));
    if (n < 0) {
        goto skip;
    }
    show("not skipped");
skip:;
    const char *late = pick(n + 1);
    if (late)
        show(late);
    return (p != NULL) + (pick(n) == NULL);
}
"#;
    let rust = same_behaviour(
        "translates_pointers_to_string_literals",
        c,
        &["0", "1", "5", "-1", ""],
    );
    assert!(rust.contains("Option<&'static [u8]>"), "{rust}");
}

#[test]
fn rejects_string_pointer_uses_it_cannot_translate() {
    let dir = scratch("rejects_string_pointer_uses_it_cannot_translate");
    // C pads to a width in bytes, Rust in characters.
    let rust = translate(
        &dir,
        "#include <stdio.h>\nint main(void) { printf(\"%5s\\n\", \"\\303\\251\"); return 0; }\n",
    );
    assert!(
        rust.contains(
            r#"compile_error!("unsupported C: printf conversion %s with flags or a width")"#
        ),
        "{rust}"
    );
    let rust = translate(&dir, "int main(void) { char *p = \"\\xff\"; return 0; }\n");
    assert!(
        rust.contains(r#"compile_error!("unsupported C: string literal that is not UTF-8")"#),
        "{rust}"
    );
    // Equal literals may or may not share an address in C.
    let rust = translate(
        &dir,
        "int main(void) { char *p = \"a\", *q = \"a\"; return p == q; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: comparison of char *")"#),
        "{rust}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_pointers_to_integer_variables() {
    let c = r#"
#include <stdio.h>
#include <stdlib.h>

void show(const int *value, unsigned char *byte) {
    if (value != NULL && byte) {
        printf("%d %u\n", *value + 1, *byte);
    } else if (!value) {
        printf("no value %d\n", value == 0);
    }
}

long twice(long *n) {
    return 2 * (*n);
}

int main(void) {
    int n = 0;
    scanf("%d", &n);
    unsigned char b = 200;
    int *p;
    int *none = (int *)0;
    show(none, &b);
    p = &n;
    show(p, &(b));
    long big = 5000000000;
    printf("%d\n", (int)(twice(&big) / 1000));
    if (n < 0) {
        goto skip;
    }
    show(&n, 0);
skip:;
    int *late = p;
    if (late)
        show(late, &b);
    return *late != 0 || (p == NULL);
}
"#;
    let rust = same_behaviour(
        "translates_pointers_to_integer_variables",
        c,
        &["0", "7", "-3", ""],
    );
    assert!(rust.contains("Option<&i32>"), "{rust}");
}

#[test]
fn rejects_integer_pointer_uses_it_cannot_translate() {
    let dir = scratch("rejects_integer_pointer_uses_it_cannot_translate");
    // No variable outlives the function for the result to borrow.
    let rust = translate(
        &dir,
        "int *pick(int *p) { return p; }\nint main(void) { int x = 1; return *pick(&x); }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: return type int *")"#),
        "{rust}"
    );
    // A pointer is read-only, so it cannot be written through.
    let rust = translate(
        &dir,
        "int main(void) { int x = 1; int *p = &x; *p = 2; return x; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: assignment to UnaryOperator")"#),
        "{rust}"
    );
    // A `char *` holds a string literal's bytes, not a character variable.
    let rust = translate(
        &dir,
        "int main(void) { char c = 'a'; char *p = &c; return 0; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: unary &")"#),
        "{rust}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_integer_division_results() {
    let c = r#"
#include <stdio.h>
#include <stdlib.h>

div_t halve(div_t d) {
    return div(d.quot, 2);
}

int main(void) {
    int x = 0, y = 1;
    long a = 0;
    scanf("%d %d", &x, &y);
    div_t r = div(x, y);
    printf("%d %d %d\n", r.quot, r.rem, r.quot * y + r.rem);
    ldiv_t l = ldiv(x, y);
    lldiv_t ll;
    ll = lldiv(x * 3LL, y);
    a = l.quot - l.rem + ll.rem;
    printf("%d %d %d\n", (int)a, (int)ll.quot, div(x, y).rem);
    printf("%d\n", halve(r).quot);
    return r.rem < 0;
}
"#;
    let rust = same_behaviour(
        "translates_integer_division_results",
        c,
        &["37 5", "-37 5", "37 -5", "-37 -5", "0 7", "2147483647 -1"],
    );
    assert!(rust.contains("(i32, i32)"), "{rust}");
}

#[test]
fn translates_program_arguments_strtol_and_static_locals() {
    let c = r#"
#include <stdio.h>
#include <stdlib.h>

int counter(int step) {
    static int total;
    static unsigned char wraps = 250;
    total += step;
    wraps++;
    if (total > 100) {
        total = 0;
    }
    return total * 1000 + wraps;
}

int other(void) {
    static long total = -5;
    total--;
    {
        int total = 7;
        {
            static int total = 1;
            total *= 3;
            printf("inner %d\n", total);
        }
        printf("shadow %d\n", total);
    }
    return (int)total;
}

int main(int argc, char *argv[]) {
    printf("argc %d\n", argc);
    for (int i = 1; i < argc; i++) {
        char *end;
        long value = strtol(argv[i], &end, 10);
        long long any = strtoll(argv[i], NULL, 0);
        int hex = strtol(argv[i], (char **)0, 16);
        printf("%s: %d %d %d %d\n", argv[i], (int)value, (int)any, hex, end == argv[i]);
        if (end != argv[i]) {
            printf("rest [%s]\n", end);
        }
        printf("%d %d\n", counter((int)value), other());
    }
    return argv[argc] == NULL;
}
"#;
    let rust = same_behaviour_on(
        "translates_program_arguments_strtol_and_static_locals",
        c,
        &[
            (&[], ""),
            (&["12", "abc", ""], ""),
            (&["  -42xyz", "0x1F", "077", "0x", "+", "-0XfG", "\t7 "], ""),
            (
                &[
                    "99999999999999999999",
                    "-99999999999999999999",
                    "9223372036854775807",
                    "-9223372036854775808",
                ],
                "",
            ),
        ],
    );
    assert!(rust.contains("AtomicU8"), "{rust}");
}

#[test]
fn rejects_static_and_argument_uses_it_cannot_translate() {
    let dir = scratch("rejects_static_and_argument_uses_it_cannot_translate");
    // A borrow of the atomic would not see later writes.
    let rust = translate(
        &dir,
        "int main(void) { static int x = 1; int *p = &x; return *p; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: unary &")"#),
        "{rust}"
    );
    let rust = translate(
        &dir,
        "#include <stdio.h>\nint main(void) { static int x; scanf(\"%d\", &x); return x; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: static local variable in this position")"#),
        "{rust}"
    );
    let rust = translate(
        &dir,
        "int main(void) { static char *s = \"a\"; return s == 0; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: static local variable of type char *")"#),
        "{rust}"
    );
    let rust = translate(
        &dir,
        "#include <stdlib.h>\nint main(int argc, char **argv) { char **p = argv; return strtol(\"1\", p, 10); }\n",
    );
    assert!(
        rust.contains(
            r#"compile_error!("unsupported C: strtol with an end pointer other than &variable")"#
        ),
        "{rust}"
    );
    let rust = translate(
        &dir,
        "int main(int argc, char **argv, char **envp) { return 0; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: main with parameters")"#),
        "{rust}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_booleans() {
    let c = r#"
#include <stdbool.h>
#include <stdio.h>

bool odd(int x) {
    return x % 2;
}

_Bool both(bool a, _Bool b) {
    return a && b;
}

int main(void) {
    int x = 0;
    unsigned char small = 0;
    bool seen;
    char *name = "set";
    scanf("%d", &x);
    small = (unsigned char)x;
    seen = false;
    bool any = x;
    bool narrow = small;
    bool named = name;
    bool cast = (bool)(x - 2);
    if (odd(x)) {
        seen = true;
    }
    while (!seen && x > 0) {
        x--;
        seen = odd(x);
    }
    printf("%d %d %d %d %d\n", seen, any, narrow, named, cast);
    printf("%d %d %d\n", both(any, cast), seen + any * 2, seen == true);
    long wide = narrow;
    return !any + (int)wide;
}
"#;
    let rust = same_behaviour(
        "translates_booleans",
        c,
        &["0", "1", "2", "4", "-3", "256", "2147483647"],
    );
    assert!(rust.contains("-> bool"), "{rust}");

    let dir = scratch("translates_booleans_rejected");
    // `++` sets a `bool` to true, and `+=` stores the sum's truth.
    let rust = translate(
        &dir,
        "#include <stdbool.h>\nint main(void) { bool b = false; b++; b += 2; return b; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: ++ on _Bool")"#),
        "{rust}"
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: += on _Bool")"#),
        "{rust}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
