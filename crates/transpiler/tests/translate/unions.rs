use crate::common::{assert_lines, check, translate};

#[test]
fn a_union_of_numbers_of_one_size_is_their_bits() {
    let rust = check(
        "union-bits",
        r#"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
union wide { double d; uint64_t u; int64_t i; };
struct cell { int tag; union { float f; uint32_t u; } v; };
static void negate(union wide *w) { w->u ^= 0x8000000000000000ull; }
static double half(const union wide *w) { return w->d / 2; }
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    float x = (float)atof(argv[1]);
    union { float flt; uint32_t num; } in;
    in.flt = x;
    uint32_t exponent = (in.num >> 23) & 0xff;
    union wide w;
    w.d = x;
    negate(&w);
    struct cell c = {0};
    c.v.f = 1.0f / 3.0f;
    printf("%08x %u %g ", in.num, exponent, half(&w));
    w.i = w.i >> 4;
    printf("%lld %08x %g\n", (long long)w.i, c.v.u, c.v.f);
    return 0;
}
"#,
        &[&["1.5"], &["-0.0"], &["3e38"], &["1e-45"], &["nan"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "v: u32,",
            "fn negate(w: &mut u64) {",
            "*w ^= 9223372036854775808;",
            "f64::from_bits(*w) / 2.0",
            "let in_: u32 = x.to_bits();",
            "let exponent: u32 = in_ >> 23 & 255;",
            "let mut w: u64 = (x as f64).to_bits();",
            "c.v = f32::to_bits(1.0f32 / 3.0f32);",
            "w = (w as i64 >> 4) as u64;",
        ],
    );
    assert!(!rust.contains("union") && !rust.contains("Wide"));
}

#[test]
fn a_member_of_a_union_that_is_more_than_read_or_stored_is_refused() {
    let refused = |name, body: &str| {
        translate(
            name,
            &format!(
                "#include <stdio.h>\nint main(void) {{\n    union {{ float f; unsigned u; }} v;\n    v.u = 1;\n    {body}\n    return v.u == 0;\n}}\n"
            ),
        )
        .unwrap_err()
    };
    assert_eq!(
        refused("union-update", "v.f += 1.0f;"),
        "unsupported: an update in place of a member of a union: no rule yet, at line 5"
    );
    assert_eq!(
        refused("union-step", "v.f++;"),
        "unsupported: an update in place of a member of a union: no rule yet, at line 5"
    );
    assert_eq!(
        refused("union-address", "if (scanf(\"%f\", &v.f) != 1) return 1;"),
        "unsupported: the address of a member of a union: no rule yet, at line 5"
    );
    assert_eq!(
        refused("union-value", "float g = v.f = 2.0f; if (g > 1) return 1;"),
        "unsupported: a store into a member of a union whose value is used: no rule yet, at line 5"
    );
}
