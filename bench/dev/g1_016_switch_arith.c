// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <limits.h>

// Function to perform all arithmetic/bitwise operations
unsigned int perform_operations(unsigned int a, unsigned int b) {
    unsigned int safe_b = b == 0 ? 1 : b;
    unsigned int shift = b % (sizeof(unsigned int) * 8); // Typically 0–31

    unsigned int add = a + b;
    unsigned int sub = a - b;
    unsigned int mul = a * b;
    unsigned int div = a / safe_b;
    unsigned int shl = a << shift;
    unsigned int shr = a >> shift;
    unsigned int xor = a ^ b;
    unsigned int and = a & b;
    unsigned int or  = a | b;
    unsigned int not_a = ~a;

    // Combine results into one final value
    return add ^ sub ^ mul ^ div ^ shl ^ shr ^ xor ^ and ^ or ^ not_a;
}

int main(int argc, char *argv[]) {
    if (argc != 2) {
        fprintf(stderr, "Usage: %s <seed>\n", argv[0]);
        return 1;
    }

    // Convert seed with error checking
    errno = 0;
    char *endptr;
    unsigned long temp_seed = strtoul(argv[1], &endptr, 10);
    if (*endptr != '\0' || errno != 0 || temp_seed > UINT_MAX) {
        fprintf(stderr, "Invalid seed: '%s'\n", argv[1]);
        return 1;
    }
    unsigned int seed = (unsigned int)temp_seed;

    // Generate random numbers
    srand(seed);
    unsigned int a = (unsigned int)rand();
    unsigned int b = (unsigned int)rand();

    // Perform all operations
    unsigned int result = perform_operations(a, b);

    // Determine which message to print
    unsigned int choice = result % 10;

    // Funny message selector
    switch (choice) {
        case 0:
            printf("Result: %u — The number is as calm as a sleeping sloth.\n", result);
            break;
        case 1:
            printf("Result: %u — It tried to divide by zero but thought better of it.\n", result);
            break;
        case 2:
            printf("Result: %u — Secretly wishes it was a float.\n", result);
            break;
        case 3:
            printf("Result: %u — Built entirely from left shifts and dreams.\n", result);
            break;
        case 4:
            printf("Result: %u — Bitwise ANDed its way into your heart.\n", result);
            break;
        case 5:
            printf("Result: %u — XOR marks the spot.\n", result);
            break;
        case 6:
            printf("Result: %u — Practically a math meme at this point.\n", result);
            break;
        case 7:
            printf("Result: %u — Stronger than a C macro on Monday morning.\n", result);
            break;
        case 8:
            printf("Result: %u — Thinks it's the boss of all unsigned ints.\n", result);
            break;
        case 9:
            printf("Result: %u — May contain traces of peanuts and logic gates.\n", result);
            break;
        default:
            printf("Result: %u — How did we even get here?\n", result);
            break;
    }

    return 0;
}

