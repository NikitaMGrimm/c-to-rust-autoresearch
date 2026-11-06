#include <stdint.h>

uint32_t rev16(uint32_t a);


uint32_t rev16(uint32_t a) {
    a = ((a & 0xAAAA) >> 1) | ((a & 0x5555) << 1);
    a = ((a & 0xCCCC) >> 2) | ((a & 0x3333) << 2);
    a = ((a & 0xF0F0) >> 4) | ((a & 0x0F0F) << 4);
    a = ((a & 0xFF00) >> 8) | ((a & 0x00FF) << 8);
    return a;
}

/* Added by the benchmark intake in place of TRACTOR's test runner. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 3) {
        return 2;
    }
    unsigned int a = (unsigned int)strtoull(argv[1], NULL, 10);
    unsigned int returns = (unsigned int)strtoull(argv[2], NULL, 10);
    returns = rev16(a);
    printf("%u\n", returns);
    return 0;
}
