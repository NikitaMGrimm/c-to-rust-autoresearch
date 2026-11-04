// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>

int main() {
    int x = 1, y = 1;
    scanf("%d %d", &x, &y);
    div_t result = div(x, y);
    printf("quotient: %d, remainder: %d\n", result.quot, result.rem);
    return 0;
}