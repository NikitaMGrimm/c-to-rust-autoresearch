// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>

void fma_array(int *out, const int *mul1, const int *mul2, const int *add, int len) {
    for (int i = 0; i < len; i++) {
        out[i] = mul1[i] * mul2[i] + add[i];
    }
}

void driver(int *out, int len) {
    fma_array(out, out, out, out, len);
    for (int i = 0; i < len; i++) {
        printf("%d\n", out[i]);
    }
}

int main() {
    int data[100];
    int i;
    for (i = 0; i < 100; i++) {
        if (scanf("%d", &data[i]) != 1) {
            break;
        }
    }

    driver(data, i);
    return 0;
}