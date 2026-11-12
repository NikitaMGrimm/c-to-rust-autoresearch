use crate::common::{assert_lines, check};

#[test]
fn globals_that_change_are_passed_as_one_struct() {
    let rust = check(
        "globals",
        r#"
#include <stdio.h>
#include <stdlib.h>
typedef struct { int floors; double baths; } house_t;
static house_t the_house = {.floors = 2, .baths = 1.5};
static const int limit = 4;
static int primes[5] = {2, 3, 5, 7, 11};
static int calls;
int next(void) {
    static int at = 0;
    calls++;
    return primes[at++ % 5];
}
void add_floor(house_t *house) { house->floors++; }
void grow(void) { if (the_house.floors < limit) add_floor(&the_house); }
int floors(void) { return the_house.floors; }
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    for (int i = 0; i < n; i++) { grow(); the_house.baths += next(); }
    printf("%d %.1f %d\n", floors(), the_house.baths, calls);
    return 0;
}
"#,
        &[&["0"], &["7"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "const LIMIT: i32 = 4;",
            "static PRIMES: [i32; 5] = [2, 3, 5, 7, 11];",
            "struct Globals {",
            "fn next(globals: &mut Globals) -> i32 {",
            "fn floors(globals: &Globals) -> i32 {",
            "add_floor(&mut globals.the_house);",
            "let globals = &mut Globals {",
        ],
    );
}
