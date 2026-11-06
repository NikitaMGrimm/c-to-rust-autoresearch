// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#define STRINGIZE(x) #x
#define TO_STRING(x) STRINGIZE(x)

int cleanup(int a, int b, int c, int d);
void print_result(const char *label, int result);
void cleanup_resources(char *dynamic_str);

int cleanup(int a, int b, int c, int d) {
    int numbers[] = {a, b, c, d};
    char *dynamic_str = NULL;
    int result = 0;

    const char *expected_str = "VALID";
    const char *input_str = "VALID";
    if (strncmp(input_str, expected_str, strlen(expected_str)) != 0) {
        printf("Input string validation failed.\n");
        goto cleanup;
    }

    for (int i = 0; i < 4; i++) {
        switch (numbers[i]) {
            case 10:
                result += 10;
            case 20:
                result += 20;
                break;
            case 30:
                result += 30;
            case 40:
                result += 40;
                break;
            default:
                result += numbers[i];
                break;
        }
    }

    dynamic_str = (char *)malloc(50 * sizeof(char));
    if (!dynamic_str) {
        printf("Memory allocation failed.\n");
        goto cleanup;
    }

    snprintf(dynamic_str, 50, "Processed numbers: %s", TO_STRING(numbers));
    printf("%s\n", dynamic_str);

cleanup:
    cleanup_resources(dynamic_str);
    return result;
}

void print_result(const char *label, int result) {
    printf("%s: %d\n", label, result);
}

void cleanup_resources(char *dynamic_str) {
    if (dynamic_str) {
        free(dynamic_str);
        dynamic_str = NULL;
    }
}

/* Added by the benchmark intake in place of TRACTOR's test runner. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 6) {
        return 2;
    }
    int a = (int)strtoll(argv[1], NULL, 10);
    int b = (int)strtoll(argv[2], NULL, 10);
    int c = (int)strtoll(argv[3], NULL, 10);
    int d = (int)strtoll(argv[4], NULL, 10);
    int returns = (int)strtoll(argv[5], NULL, 10);
    returns = cleanup(a, b, c, d);
    printf("%d\n", returns);
    return 0;
}
