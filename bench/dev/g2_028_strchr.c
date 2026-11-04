// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <string.h>

int foo(const char *in, char c) {
    int res = 0;
    for (const char *s = in; s = strchr(s, c); s++) {
        res++;
    }
    return res;
}

void driver(const char *in) {
    printf("A: %d\n", foo(in, 'A'));
    printf("x: %d\n", foo(in, 'x'));
}

int main() {
    char in[1000] = "";
    fread(in, 1, sizeof(in), stdin);
    driver(in);
    return 0;
}
