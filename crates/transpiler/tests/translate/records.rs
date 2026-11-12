use crate::common::{assert_lines, check};

#[test]
fn records_and_arrays_hold_values() {
    let rust = check(
        "records",
        r#"
#include <stdio.h>
#include <stdlib.h>
typedef struct { int x; int y; } point_t;
struct line { point_t from, to; double weight; int marks[3]; };
point_t shifted(point_t p, int by) { p.x += by; return p; }
int length2(struct line l) {
    int dx = l.to.x - l.from.x, dy = l.to.y - l.from.y;
    return dx * dx + dy * dy;
}
int main(int argc, char *argv[]) {
    int n = atoi(argv[1]);
    point_t origin = {0};
    struct line l = {{1, 2}, {.x = n, .y = 4}, 0.5, {7}};
    point_t copy = l.to;
    copy.y = 99;
    l.from = shifted(origin, n);
    int table[2][3] = {{1, 2, 3}, {4, 5, 6}};
    unsigned char bytes[4] = {1, 2};
    long total = 0;
    for (int i = 0; i < 2; i++)
        for (int j = 0; j < 3; j++)
            total += table[i][j] * l.marks[j];
    l.marks[n % 3] = bytes[1];
    printf("%d %d %d %ld %d %.1f %lu\n", length2(l), copy.y, l.to.y, total, l.marks[n % 3], l.weight,
           sizeof(table) / sizeof(table[0]));
    return 0;
}
"#,
        &[&["0"], &["5"]],
        "",
    );
    assert_lines(
        &rust,
        &[
            "#[derive(Clone, Copy, Default)]",
            "struct PointT {",
            "fn shifted(mut p: PointT, by: i32) -> PointT {",
            "let origin: PointT = PointT { x: 0, y: 0 };",
            "let table: [[i32; 3]; 2] = [[1, 2, 3], [4, 5, 6]];",
        ],
    );
}
