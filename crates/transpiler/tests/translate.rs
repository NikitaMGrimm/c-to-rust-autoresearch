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
            .arg("-lm")
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
        "#include <stdio.h>\nint main(void) { printf(\"%a\\n\", 1.5); return 0; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: printf conversion %a")"#),
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
    assert!(rust.contains("Option<&'a [u8]>"), "{rust}");
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
    assert!(rust.contains("Option<&[std::cell::Cell<i32>]>"), "{rust}");
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
    // A pointer is the cells from where it points, so it cannot move.
    let rust = translate(
        &dir,
        "int main(void) { int a[2] = {1, 2}; int *p = a + 1; return *p; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: binary + on int *")"#),
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

#[test]
fn translates_unsigned_and_bitwise_arithmetic_and_switch() {
    let c = r#"
#include <stdio.h>

static unsigned int mix(unsigned int a, unsigned int b) {
    static unsigned int total = 1;
    unsigned int shift = b % (sizeof(unsigned int) * 8);
    unsigned long wide = (unsigned long)a * 4294967296u + b;
    total *= a + 1;
    total ^= b;
    total <<= 1;
    a -= b;
    a += 7u;
    a *= 3u;
    a |= b & 0xf0u;
    a >>= 1;
    printf("%u %u %u\n", a << shift, a >> shift, ~a);
    printf("%u %u %u\n", (unsigned int)(wide >> 32), (unsigned int)wide, -a);
    return (a ^ b) + total + (b == 0 ? 1 : a / b) + (b != 0 ? a % b : 0u);
}

static const char *name(int n) {
    switch (n) {
    case -1:
        return "minus one";
    case 'a':
    case 'c':
        return "letter";
    default:
        return "other";
    case 0:
        return "zero";
    }
}

int main(void) {
    int x, y;
    while (scanf("%d %d", &x, &y) == 2) {
        int signed_mix = (x & 0x0f) | (y << 3) ^ (x >> 2);
        signed_mix ^= ~y;
        signed_mix &= 0x7fff;
        printf("%d %u\n", signed_mix, mix((unsigned int)x, (unsigned int)y));
        printf("%s %s\n", name(x), name(y));
        for (int i = 0; i < 4; i++) {
            switch (i + x % 3) {
            case 1:
                if (y > 5) {
                    break;
                }
                printf("one");
                break;
            case 2:
                continue;
            case 3: {
                switch (y & 1) {
                case 0:
                    printf("even");
                    break;
                }
                break;
            }
            default:
                printf("[%d]", i);
            }
            printf(" %d;", i);
        }
        printf("\n");
    }
    switch (x) {
    }
    return sizeof x + sizeof(long) + sizeof(char *);
}
"#;
    let rust = same_behaviour(
        "translates_unsigned_and_bitwise_arithmetic_and_switch",
        c,
        &[
            "0 0\n",
            "1 2\n97 99\n-1 7\n",
            "-2147483648 31\n2147483647 -1\n12345 678\n",
            "4 33\n-7 100\n",
        ],
    );
    assert!(rust.contains("wrapping_mul"), "{rust}");
    assert!(rust.contains("match"), "{rust}");
}

#[test]
fn translates_rand_strtoul_errno_and_fprintf() {
    let c = r#"
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    printf("%d %d\n", rand(), rand());
    for (int i = 1; i < argc; i++) {
        char *end;
        errno = 0;
        unsigned long u = strtoul(argv[i], &end, 0);
        int range = errno;
        errno = 0;
        long l = strtol(argv[i], NULL, 0);
        fprintf(stdout, "%u %u %d %d %d [%s]\n", (unsigned int)(u >> 32), (unsigned int)u, range,
                (int)(l >> 32), errno, end);
        if (*end != '\0') {
            fprintf(stderr, "trailing %s\n", end);
            continue;
        }
        srand((unsigned int)u);
        printf("%d %d %d\n", rand(), rand(), rand() % 100);
    }
    return errno == ERANGE;
}
"#;
    same_behaviour_on(
        "translates_rand_strtoul_errno_and_fprintf",
        c,
        &[
            (&[], ""),
            (&["0", "1", "2", "42", "2147483648", "4294967295"], ""),
            (&["-1", " +17", "0x10x", "", "18446744073709551615"], ""),
            (
                &[
                    "18446744073709551616",
                    "-18446744073709551616",
                    "9223372036854775808",
                ],
                "",
            ),
            (&["-9223372036854775809", "-9223372036854775808"], ""),
        ],
    );
}

#[test]
fn rejects_switches_it_cannot_translate() {
    let dir = scratch("rejects_switches_it_cannot_translate");
    let cases = [
        (
            "int main(void) { int x = 1; switch (x) { case 1: x++; case 2: x++; } return x; }",
            "switch case that falls through",
        ),
        (
            "int main(void) { int x = 1; switch (x) { case 1: if (x) { case 2: x++; } } return x; }",
            "case label inside another statement",
        ),
        (
            "int main(void) { int x = 1; switch (x) { case 1:; int y = 1; return y; case 2: y = 2; return y; } return x; }",
            "variable declared in one case and used in another",
        ),
        (
            "int main(void) { int x = 1; switch (x) { x++; case 1: return 1; } return x; }",
            "statement before the first case of a switch",
        ),
        (
            "int main(void) { int x = 1; switch (x) { case 1 ... 3: return 1; } return x; }",
            "case range",
        ),
        (
            "#include <stdio.h>\nint main(void) { FILE *f = stdout; fprintf(f, \"x\"); return 0; }",
            "fprintf to a stream other than stdout or stderr",
        ),
    ];
    for (c, error) in cases {
        let rust = translate(&dir, c);
        assert!(
            rust.contains(&format!(r#"compile_error!("unsupported C: {error}")"#)),
            "{c}\n{rust}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_integer_arrays_and_pointers_into_them() {
    let c = r#"
#include <stdio.h>

static void bump(int *restrict p, unsigned char *b) {
    (*p)++;
    *p += 10;
    *b = *b + 100;
    p[0] *= 2;
}

static long sum(const long *xs, int n) {
    long total = 0;
    for (int i = 0; i < n; i++)
        total += xs[i];
    return total;
}

static void fma(int *out, const int *mul, const int *add, int len) {
    for (int i = 0; i < len; i++) {
        out[i] = out[i] * mul[i] + add[i];
    }
}

static int twice(int v) {
    int *const p = &v;
    *p *= 2;
    return v;
}

static int skip(int n) {
    int k = 4;
    int *pk = &k;
    if (n == 7) goto done;
    *pk = 9;
done:
    return k;
}

int main(void) {
    int n = 0;
    if (scanf("%d", &n) != 1 || n < 1 || n > 50) {
        return 1;
    }
    int x = n;
    int *px = &x;
    unsigned char b = 200;
    bump(px, &b);
    printf("%d %d %u\n", x, *px, b);
    x = 3;
    printf("%d %d\n", *px + x, twice(x));

    int a[6] = {[4] = 7, 1};
    long ls[4] = {1, 2, 3};
    int zeros[5] = {0};
    printf("%d %d %d %d\n", a[0], a[4], a[5], zeros[4]);
    printf("%d %d\n", (int)sum(ls, 4), (int)(sizeof a / sizeof a[0]));

    int v[n];
    for (int i = 0; i < n; i++) {
        if (scanf("%d", &v[i]) != 1) {
            v[i] = i;
        }
    }
    fma(v, v, v, n);
    fma(&a[2], &a[3], zeros, 3);
    printf("%d %d %d %d\n", v[0], v[n - 1], a[2], a[5]);
    int *end = &a[6];
    if (end) {
        printf("end\n");
    }
    unsigned u[3];
    u[0] = 0;
    u[0] -= 1;
    u[1] = 7;
    u[1]++;
    printf("%u %u\n", u[0], u[1]);
    // C leaves these undefined, and Rust panics only if they run.
    if (n > 1000) {
        a[6] = 1;
        printf("%d\n", a[10]);
    }
    printf("%d\n", skip(n));
    return a[0];
}
"#;
    let rust = same_behaviour(
        "translates_integer_arrays_and_pointers_into_them",
        c,
        &["", "0", "1 5", "3 1 2 3", "7 -1 -2", "50"],
    );
    assert!(rust.contains("&vec![std::cell::Cell::new(0); "), "{rust}");
}

#[test]
fn translates_char_arrays_and_the_string_functions_that_write_them() {
    let c = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void show(const char *line) {
    if (line != NULL) {
        printf("[%s] %d\n", line, (int)strlen(line));
    }
}

/* Returns one of its arguments, so both borrow the same arrays. */
const char *longer(const char *a, const char *b) {
    return strlen(a) >= strlen(b) ? a : b;
}

int main(void) {
    char line[8] = "";
    char word[6] = "ab";
    char full[2] = "xy";
    char list[5] = {'o', 'k', 0};
    int n = 3;
    char vla[n];
    int total = 0;
    show(word);
    show(list);
    printf("%d %d %d\n", full[0], full[1], word[5]);
    while (fgets(line, sizeof(line), stdin) != NULL) {
        total += atoi(line);
        printf("%d %d <%s>\n", line[0], (int)atol(line), line);
    }
    printf("total %d\n", total);
    if (fgets(line, 1, stdin) != NULL) {
        printf("n=1 stores %d\n", line[0]);
    }
    if (fgets(line, 0, stdin) == NULL) {
        printf("n=0 fails\n");
    }
    memset(word, 'z', 3);
    word[1] = 'Y';
    show(word);
    strncpy(line, "abcdef", 3);
    line[3] = '\0';
    show(line);
    strncpy(line, "q", 5);
    printf("%d %d %d\n", line[0], line[1], line[4]);
    strcpy(line, "copied");
    show(longer(line, "lit"));
    show(longer("a much longer literal", line));
    vla[0] = 'v';
    vla[1] = 0;
    show(vla);
    const char *p = line;
    printf("%d %d\n", p[2], *p);
    return line[0] - 'c';
}
"#;
    let rust = same_behaviour(
        "translates_char_arrays_and_the_string_functions_that_write_them",
        c,
        &[
            "",
            "12\n-3\n",
            "  42 tail\nlonger than the buffer\n7",
            "5\nx",
        ],
    );
    // A literal copies into the array's bytes, which fgets fills in place.
    assert!(rust.contains("copy_from_slice(b\"ab\")"), "{rust}");
    assert!(rust.contains("crt::fgets(&mut line[..], "), "{rust}");
    // Every `char *` of a function borrows for one lifetime.
    assert!(
        rust.contains(
            "fn longer<'a>(a: Option<&'a [u8]>, b: Option<&'a [u8]>) -> Option<&'a [u8]>"
        ),
        "{rust}"
    );
}

#[test]
fn reads_with_fgets_and_scanf_from_one_input() {
    let c = r#"
#include <stdio.h>

int main(void) {
    char rest[16];
    int x = 0;
    scanf("%d", &x);
    if (fgets(rest, 16, stdin) != NULL) {
        printf("%d <%s>\n", x, rest);
    }
    if (fgets(rest, 16, stdin) != NULL) {
        printf("<%s>\n", rest);
    }
    return 0;
}
"#;
    same_behaviour(
        "reads_with_fgets_and_scanf_from_one_input",
        c,
        &["7 after\nnext\n", "7\n", "9"],
    );
}

#[test]
fn rejects_array_uses_it_cannot_translate() {
    let dir = scratch("rejects_array_uses_it_cannot_translate");
    let cases = [
        // Clang gives the length only as text, so it must be a name.
        (
            "int main(void) { int n = 2; int a[n + 1]; a[0] = 0; return a[0]; }",
            "variable length array int[n + 1]",
        ),
        (
            "int main(void) { int a[2][3]; return 0; }",
            "type int[2][3]",
        ),
        // A `char *` only reads the bytes it points to.
        (
            "#include <stdio.h>\nvoid f(char *s) { fgets(s, 4, stdin); }",
            "fgets into anything but a local char array",
        ),
        (
            "#include <stdio.h>\n#include <string.h>\nint main(void) { char s[4]; char *p = s; strcpy(p, \"ab\"); return 0; }",
            "strcpy into anything but a local char array",
        ),
        (
            "#include <stdio.h>\nint main(void) { char s[4]; fgets(s, 4, stderr); return 0; }",
            "fgets from a stream other than stdin",
        ),
        (
            "int main(void) { int a[2]; if (a[0]) goto end; int b[2]; end: return 0; }",
            "declaration in a block with labels",
        ),
    ];
    for (c, error) in cases {
        let rust = translate(&dir, c);
        assert!(
            rust.contains(&format!(r#"compile_error!("unsupported C: {error}")"#)),
            "{c}\n{rust}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_floating_point_values() {
    let c = r#"
#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

double mean(double a, float b) {
    return (a + b) / 2;
}

void power(double x, double y) {
    errno = 0;
    double r = pow(x, y);
    printf("pow %g %d\n", r, errno);
}

int main(int argc, char **argv) {
    char *end;
    for (int i = 1; i < argc; i++) {
        errno = 0;
        double x = strtod(argv[i], &end);
        printf("%g|%.3e|%f|%d|%s|", x, x, x, errno, end);
        printf("%-12.4G|%10.2f|%E|%lf|%.0f|%.g\n", x, -x, x * 1e10, x / 3, x, x);
    }
    double base = argc > 1 ? atof(argv[1]) : 0.1;
    float single = 1.0F / 3;
    single += 0.25F;
    double wide = single;
    // C leaves converting a value out of range undefined.
    long whole = fabs(base) < 1e6 ? (long)(base * 1000.5) : 0;
    unsigned u = 7;
    double sum = u + whole + 0.125;
    sum -= 1;
    sum *= 2.5;
    sum /= -4;
    printf("%.9g %.17g %d %g %d\n", single, wide, (int)whole, sum, fabs(sum) < 1e9 ? (int)-sum : 0);
    printf("%g %g %g %g\n", fabs(-base), floor(-2.5), ceil(2.25), mean(base, single));
    errno = 0;
    printf("%g %d\n", sqrt(base), errno);
    power(2, 0.5);
    power(-2, 0.5);
    power(0, -1);
    power(10, 400);
    power(10, -400);
    power(10, -320);
    power(base, 3);
    if (base) {
        printf("nonzero %d %d %d\n", base < 1.5, !base, base != 2);
    }
    printf("%d %g %d %d\n", base == base, -0.0, (int)sizeof(double), (int)sizeof single);
    return base >= 1;
}
"#;
    let numbers = [
        "2.5",
        "-0.1",
        "1e500",
        "-1e500",
        "1e-400",
        "2e-310",
        "0e-999",
        "inf",
        "-INFINITY",
        "nan(abc)",
        "nan(",
        "-nan",
        ".5",
        "5.",
        ".",
        "1e",
        "1e+",
        "  +3.25xyz",
        "x",
        "-x",
        "123456789",
        "0.0001",
        "0.00001",
        "100000",
        "1000000",
        "2.675",
        "0.125",
    ];
    let mut runs: Vec<(&[&str], &str)> = numbers
        .iter()
        .map(|number| (std::slice::from_ref(number), ""))
        .collect();
    runs.push((&[], ""));
    runs.push((&numbers[..], ""));
    let rust = same_behaviour_on("translates_floating_point_values", c, &runs);
    assert!(rust.contains("let mut sum: f64"), "{rust}");
    assert!(rust.contains("0.25_f32"), "{rust}");
}

#[test]
fn rejects_floating_uses_it_cannot_translate() {
    let dir = scratch("rejects_floating_uses_it_cannot_translate");
    let rust = translate(
        &dir,
        "int main(void) { float f = 1; f += 0.5; return f; }\n",
    );
    assert!(
        rust.contains(r#"compile_error!("unsupported C: += on float")"#),
        "{rust}"
    );
    let rust = translate(
        &dir,
        "#include <stdio.h>\nint main(void) { printf(\"%+f %.2d\\n\", 1.0, 3); return 0; }\n",
    );
    assert!(
        rust.contains(
            r#"compile_error!("unsupported C: printf floating conversion with the flag + or 0")"#
        ),
        "{rust}"
    );
    let rust = translate(
        &dir,
        "#include <stdio.h>\nint main(void) { printf(\"%.2d\\n\", 3); return 0; }\n",
    );
    assert!(
        rust.contains(
            "unsupported C: printf precision or length on a conversion that is not floating"
        ),
        "{rust}"
    );
    let rust = translate(
        &dir,
        "int main(void) { long double x = 1.5L; return x > 1; }\n",
    );
    assert!(rust.contains("unsupported C: type long double"), "{rust}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_structs_and_bit_fields() {
    let c = r#"
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>

struct point { int x; int type; };
typedef struct point point_t;
typedef struct { struct point corner; long area; bool filled; } rect;
typedef struct { unsigned a : 3; int s : 4; unsigned char c : 2; bool b : 1; } bits;
typedef struct { int v; } Option;

static struct point make(int x, int y) {
    struct point p = {.type = y, .x = x};
    return p;
}

static void shift(point_t *p, int dx) {
    p->x += dx;
    (*p).type -= dx;
}

static int area_of(rect r) {
    r.area = (long)r.corner.x * r.corner.type;
    return (int)r.area;
}

static void bump(int *v) {
    (*v)++;
}

static void show(const rect *r) {
    if (r == NULL) {
        printf("null\n");
        return;
    }
    printf("%d %d %d %d\n", r->corner.x, r->corner.type, (int)r->area, r->filled);
}

int main(void) {
    unsigned u = 0;
    int n = 0;
    scanf("%u %d", &u, &n);
    printf("u=%u\n", u);
    struct point a = make(n, 3);
    point_t b = a;
    b.x = 100;
    shift(&a, 2);
    printf("%d %d %d %d\n", a.x, a.type, b.x, b.type);
    rect r = {{4, 5}, 0, true};
    rect r2;
    r2 = r;
    r2.corner.x = 7;
    printf("%d %d\n", area_of(r), area_of(r2));
    show(&r);
    show(NULL);
    bump(&r.corner.type);
    scanf("%d", &r2.corner.type);
    r.corner = r2.corner;
    show(&r);
    show(&r2);
    Option o;
    o.v = n;
    printf("%d\n", o.v);
    bits k = {0};
    k.a = u;
    k.s = n;
    k.c = 7;
    k.b = n;
    printf("%u %d %d %d\n", k.a, k.s, k.c, k.b);
    for (int i = 0; i < 9; i++) {
        k.a++;
        k.s++;
    }
    k.s--;
    printf("%u %d\n", k.a, k.s);
    k.a = k.a + 6;
    printf("%u\n", k.a);
    return a.x;
}
"#;
    let rust = same_behaviour(
        "translates_structs_and_bit_fields",
        c,
        &[
            "5 9 11",
            "4294967295 -3 2",
            "-1 7 0",
            // `%u` reads as `strtoul` does and keeps the low 32 bits.
            "9223372036854775808 15 4",
            "18446744073709551616 -8 3",
            "-18446744073709551615 2 2",
            "-9223372036854775809 1 1",
            "+7 x",
            "",
        ],
    );
    // Members are cells, so a struct is never `mut`, and only a struct that
    // is assigned whole, and those inside it, need a `set`.
    assert!(!rust.contains("let mut a"), "{rust}");
    assert!(rust.contains("impl rect {"), "{rust}");
    assert!(rust.contains("impl point {"), "{rust}");
    assert!(!rust.contains("impl bits {"), "{rust}");
    assert!(rust.contains("struct Option_ {"), "{rust}");
}

#[test]
fn translates_structs_in_a_block_with_labels() {
    let c = r#"
#include <stdio.h>

struct acc { int total; unsigned steps : 3; };

static void add(struct acc *a, int v) {
    a->total += v;
    a->steps++;
}

int main(void) {
    int n = 0;
    scanf("%d", &n);
    struct acc a = {0};
again:
    add(&a, n);
    if (a.total < 20) goto again;
    struct acc b = a;
    printf("%d %u %d\n", b.total, b.steps, a.total);
    return 0;
}
"#;
    same_behaviour(
        "translates_structs_in_a_block_with_labels",
        c,
        &["1", "3", "7"],
    );
}

#[test]
fn rejects_struct_uses_it_cannot_translate() {
    let dir = scratch("rejects_struct_uses_it_cannot_translate");
    let cases = [
        (
            "struct a { int v[2]; }; int main(void) { struct a x; return 0; }",
            "type struct a",
        ),
        (
            "union u { int a; }; int main(void) { union u x; x.a = 1; return x.a; }",
            "type union u",
        ),
        // An unnamed bit-field has no member to name in Rust.
        (
            "struct p { int x : 3; int : 2; }; int main(void) { struct p a = {1}; return a.x; }",
            "type struct p",
        ),
        (
            "struct p { int x; }; int main(void) { return sizeof(struct p); }",
            "sizeof struct p",
        ),
        (
            "struct p { int x; }; struct p *f(struct p *q) { return q; }",
            "return type struct p *",
        ),
        (
            "struct p { int x; }; int f(struct p *a, struct p *b) { return a == b; }",
            "comparison of struct p *",
        ),
        (
            "struct p { int x; }; int main(void) { struct p a[2]; return 0; }",
            "type struct p[2]",
        ),
        // C computes it in `int`, which the bit-field would need converting.
        (
            "struct p { unsigned x : 3; }; int main(void) { struct p a = {1}; a.x += 1; return a.x; }",
            "+= on unsigned int",
        ),
    ];
    for (c, error) in cases {
        let rust = translate(&dir, c);
        assert!(
            rust.contains(&format!(r#"compile_error!("unsupported C: {error}")"#)),
            "{c}\n{rust}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn translates_global_variables() {
    let c = r#"
#include <stdio.h>

struct point { int x; double y; };

static struct point origin = {.y = 1.5};
int counter;
int counter = 3;
static long total = 10;
static unsigned char flags;
double scale = 2;
_Bool ready = 1;
static int table[5] = {1, 2, 3};
int zeros[3];

static void bump(int *p) { (*p)++; }

static void move(struct point *p, int dx) { p->x += dx; }

static int sum(const int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

int main(void) {
    int n = 0;
    if (scanf("%d", &n) != 1) return 1;
    counter += n;
    bump(&counter);
    total *= counter;
    flags = flags - 1;
    scale = scale * 1.5;
    table[3] = n;
    zeros[1]++;
    move(&origin, n);
    origin.y += 1;
    struct point copy = origin;
    if (ready) printf("%d %d %d %u %.2f\n", counter, copy.x, (int)total, flags, scale);
    int counter = 7;
    printf("%d %d %d %d\n", counter, sum(table, 5), zeros[1], origin.x);
    scanf("%d", &zeros[2]);
    printf("%d %.1f\n", zeros[2], origin.y);
    return 0;
}
"#;
    same_behaviour("translates_global_variables", c, &["4 9", "-3 2", "5"]);
}

#[test]
fn rejects_global_variables_it_cannot_translate() {
    let dir = scratch("rejects_global_variables_it_cannot_translate");
    let cases = [
        (
            "char name[8]; int main(void) { return 0; }",
            "global variable of type char[8]",
        ),
        (
            "int x; int *p = &x; int main(void) { return 0; }",
            "global variable of type int *",
        ),
        (
            "int grid[2][2]; int main(void) { return 0; }",
            "global variable of type int[2][2]",
        ),
    ];
    for (c, error) in cases {
        let rust = translate(&dir, c);
        assert!(
            rust.contains(&format!(r#"compile_error!("unsupported C: {error}")"#)),
            "{c}\n{rust}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
