// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#include <errno.h>

//Takes two arguments, a base and an exponent, and prints base^exponent
int main(int argc, char *argv[]) {
    if (argc != 3) {
        fprintf(stderr, "Usage: %s base exponent\n", argv[0]);
        return 1;
    }

    char *endptr1, *endptr2;

    // Convert base
    errno = 0;
    double base = strtod(argv[1], &endptr1);
    if (errno == ERANGE) {
        fprintf(stderr, "Range error while converting base '%s'\n", argv[1]);
        return 1;
    } else if (*endptr1 != '\0') {
        fprintf(stderr, "Invalid numeric input for base: '%s'\n", argv[1]);
        return 1;
    }

    // Convert exponent
    errno = 0;
    double exponent = strtod(argv[2], &endptr2);
    if (errno == ERANGE) {
        fprintf(stderr, "Range error while converting exponent '%s'\n", argv[2]);
        return 1;
    } else if (*endptr2 != '\0') {
        fprintf(stderr, "Invalid numeric input for exponent: '%s'\n", argv[2]);
        return 1;
    }

    // Calculate power
    errno = 0;
    double result = pow(base, exponent);
    if (errno == EDOM) {
        fprintf(stderr, "Domain error: pow(%.2f, %.2f) is undefined in the real number domain.\n", base, exponent);
        return 1;
    } else if (errno == ERANGE) {
        fprintf(stderr, "Range error: pow(%.2f, %.2f) caused overflow or underflow.\n", base, exponent);
        return 1;
    }

    printf("Result: %.2f\n", result);
    return 0;
}

