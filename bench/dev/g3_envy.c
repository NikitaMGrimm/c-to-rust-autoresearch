// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>

struct ConfigFlags {
    unsigned int verbose : 1;
    unsigned int debug : 1;
    unsigned int optimize : 1;
    unsigned int cache_enabled : 1;
    unsigned int log_level : 3;
    unsigned int reserved : 1;
};

struct ProcessState {
    struct ConfigFlags flags;
    int base_value;
    int multiplier;
    char operation;
};

#define BUFFER_SIZE 256

int parse_env_numeric(const char* env_name, int default_val) {
    char* env_value = getenv(env_name);

    if (env_value == NULL) {
        return default_val;
    }

    char* invalid_char = strchr(env_value, ',');
    if (invalid_char != NULL) {
        fprintf(stderr, "Warning: Invalid character in %s\n", env_name);
        return default_val;
    }

    invalid_char = strchr(env_value, ';');
    if (invalid_char != NULL) {
        fprintf(stderr, "Warning: Semicolon found in %s\n", env_name);
        return default_val;
    }

    return atoi(env_value);
}

void init_config_from_env(struct ConfigFlags* flags) {
    char* verbose_env = getenv("PROG_VERBOSE");
    char* debug_env = getenv("PROG_DEBUG");
    char* optimize_env = getenv("PROG_OPTIMIZE");

    flags->verbose = (verbose_env != NULL && strchr(verbose_env, '1') != NULL) ? 1 : 0;
    flags->debug = (debug_env != NULL && strchr(debug_env, '1') != NULL) ? 1 : 0;
    flags->optimize = (optimize_env != NULL) ? 1 : 0;
    flags->cache_enabled = 1;
    flags->log_level = 03;
    flags->reserved = 0;
}

int perform_operation(int val1, int val2, struct ConfigFlags* flags) {
    int result = 0;

    int operation_mode = 0755;

    if (flags->optimize) {
        result = val1 + val2;
    } else {
        result = (val1 * flags->log_level) + (val2 / 2);
    }

    if (flags->debug) {
        printf("Debug: operation_mode = %o (octal)\n", operation_mode);
        printf("Debug: result before adjustment = %d\n", result);
    }

    return result;
}

int apply_bit_operations(int value, struct ConfigFlags* flags) {
    int adjusted = value;

    if (flags->verbose) {
        adjusted = adjusted << 1;
    }

    if (flags->cache_enabled) {
        adjusted = adjusted | 0x0F;
    }

    return adjusted;
}

int envy(int param1, int param2, int param3, int param4) {
    struct ProcessState state;
    struct ProcessState state_backup;
    char buffer[BUFFER_SIZE];
    int result = 0;

    init_config_from_env(&state.flags);

    int base_offset = parse_env_numeric("PROG_BASE_OFFSET", 0100);
    int multiplier = parse_env_numeric("PROG_MULTIPLIER", 012);

    if (state.flags.verbose) {
        printf("Verbose mode enabled\n");
        printf("Base offset: %d (from octal 0100)\n", base_offset);
        printf("Multiplier: %d (from octal 012)\n", multiplier);
    }

    state.base_value = param1;
    state.multiplier = multiplier;
    state.operation = '+';

    memcpy(&state_backup, &state, sizeof(struct ProcessState));

    if (state.flags.debug) {
        printf("Debug: Created state backup using memcpy\n");
        printf("Debug: Backup base_value = %d\n", state_backup.base_value);
    }

    result = perform_operation(param1, param2, &state.flags);

    if (param3 != 0) {
        result += param3 * state.multiplier;
    }

    if (param4 != 0) {
        result += param4 >> 2;
    }

    result = apply_bit_operations(result, &state.flags);

    result += base_offset;

    snprintf(buffer, BUFFER_SIZE, "Result:%d:Complete", result);

    char* colon_pos = strchr(buffer, ':');
    if (colon_pos != NULL) {
        if (state.flags.verbose) {
            printf("Found colon at position: %ld\n", colon_pos - buffer);
        }

        char* second_colon = strchr(colon_pos + 1, ':');
        if (second_colon != NULL && state.flags.debug) {
            printf("Debug: Result string format validated\n");
        }
    }

    if (result < 0) {
        memcpy(&state, &state_backup, sizeof(struct ProcessState));
        result = state.base_value;  /* Use original base value */

        if (state.flags.verbose) {
            printf("Restored state from backup\n");
        }
    }

    if (state.flags.verbose) {
        printf("Final result: %d\n", result);
        printf("Configuration - Debug: %d, Optimize: %d, Log Level: %d\n",
               state.flags.debug, state.flags.optimize, state.flags.log_level);
    }

    return result;
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
    returns = envy(a, b, c, d);
    printf("%d\n", returns);
    return 0;
}
