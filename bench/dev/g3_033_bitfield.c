// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdbool.h>
#include <stdio.h>

typedef struct {
    unsigned int x : 2;
    unsigned int y : 3;
    bool b : 1;
    int z;
} foo_t;

void print_foo(const foo_t *foo) {
    printf("%u %u %d %d\n", foo->x, foo->y, foo->b, foo->z);
}

void driver(unsigned int x, unsigned int y, bool b, int z) {
    foo_t foo = {.x = x, .y = y, .b = b, .z = z};
    print_foo(&foo);
}

int main() {
    unsigned int x = 0, y = 0;
    int b = 0, z = 0;
    scanf("%u", &x);
    scanf("%u", &y);
    scanf("%d", &b);
    scanf("%d", &z);
    driver(x, y, !!b, z);
    return 0;
}