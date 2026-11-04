// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>

void fma_array(int *restrict out, const int *mul1, const int *mul2, const int *add, int len) {
    for (int i = 0; i < len; i++) {
        out[i] = mul1[i] * mul2[i] + add[i];
    }
}

int call_fma(const int *data, int len) {
    if (len == 0) return 0;
    int out[len];
    int ones[len];
    int zeros[len];

    out[0] = 0;
    for (int i = 0; i < len; i++) {
        ones[i] = 1;
        zeros[i] = 0;
    }

    fma_array(out, ones, data, zeros, len);
    return out[len-1];
}

int main() {
    int data[100];
    int i;
    for (i = 0; i < 100; i++) {
        if (scanf("%d", &data[i]) != 1) {
            break;
        }
    }

    int result = call_fma(data, i);
    printf("%d\n", result);

    return 0;
}