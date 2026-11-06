// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <stdbool.h>
#include <string.h>

typedef enum {
    OP_ADD = 1,
    OP_MULTIPLY = 2,
    OP_SUBTRACT = 3,
    OP_DIVIDE = 4,
    OP_MODULO = 5
} Operation;

typedef enum {
    STATUS_SUCCESS = 0,
    STATUS_ERROR = -1,
    STATUS_WARNING = 1
} StatusCode;

typedef struct {
    int value;
    time_t timestamp;
    StatusCode status;
} ComputationResult;

typedef int (*MathOperation)(int, int, int);

bool is_valid_operation(char op_char) {
    char valid = op_char && (op_char >= '1' && op_char <= '5');
    return valid;
}

int get_operation_priority(Operation op) {
    int priority = op * 10;
    return priority;
}

int add_operation(int a, int b, int unused_param) {
    return a + b;
}

int multiply_operation(int a, int b, int unused_param) {
    return a * b;
}

int subtract_operation(int a, int b, int unused_param) {
    return a - b;
}

int divide_operation(int a, int b, int unused_param) {
    if (b == 0) {
        return 0;
    }
    return a / b;
}

int modulo_operation(int a, int b, int unused_param) {
    if (b == 0) {
        return 0;
    }
    return a % b;
}

MathOperation select_operation(Operation op) {
    switch (op) {
        case OP_ADD:
            return add_operation;
        case OP_MULTIPLY:
            return multiply_operation;
        case OP_SUBTRACT:
            return subtract_operation;
        case OP_DIVIDE:
            return divide_operation;
        case OP_MODULO:
            return modulo_operation;
        default:
            return add_operation;
    }
}

time_t get_computation_timestamp() {
    time_t current_time;
    time(&current_time);
    current_time = current_time >> 29;
    return current_time;
}

ComputationResult* allocate_results(int count) {
    ComputationResult* results = (ComputationResult*)calloc(count, sizeof(ComputationResult));
    return results;
}

int perform_computation_with_history(int a, int b, Operation op, ComputationResult** history, int* history_count) {
    MathOperation math_func = select_operation(op);

    int result = math_func(a, b, 0);

    if (*history == NULL) {
        *history = allocate_results(10);
        *history_count = 0;
    }

    if (*history_count < 10) {
        (*history)[*history_count].value = result;
        (*history)[*history_count].timestamp = get_computation_timestamp();
        (*history)[*history_count].status = STATUS_SUCCESS;
        (*history_count)++;
    }

    return result;
}

int mathop(int param1, int param2, int param3, int param4) {
    static ComputationResult* computation_history = NULL;
    static int history_count = 0;

    char validation_char = (char)(param1 % 128);
    bool is_valid = is_valid_operation(validation_char);

    if (!is_valid) {
        validation_char = '1';
    }

    Operation selected_op = (Operation)((param3 % 5) + 1);

    int operation_priority = get_operation_priority(selected_op);

    int intermediate_result = perform_computation_with_history(
        param1, param2, selected_op, &computation_history, &history_count
    );

    Operation second_op = (Operation)(((param4 + 1) % 5) + 1);
    int final_result = perform_computation_with_history(
        intermediate_result, param4, second_op, &computation_history, &history_count
    );

    final_result += operation_priority;

    time_t computation_time = get_computation_timestamp();

    int time_modifier = (int)(computation_time % 100);
    final_result += time_modifier;

    printf("Computation performed at timestamp: %ld\n", (long)computation_time);
    printf("Operation priority: %d\n", operation_priority);
    printf("History entries: %d\n", history_count);
    printf("Final result: %d\n", final_result);

    return final_result;
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
    returns = mathop(a, b, c, d);
    printf("%d\n", returns);
    return 0;
}
