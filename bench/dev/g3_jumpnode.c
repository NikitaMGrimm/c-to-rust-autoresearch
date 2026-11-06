// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

typedef struct {
    int id;
    int parent_id;
    double value;
    int data[4];
} Node;

#define MAX_NODES 100
static Node node_storage[MAX_NODES];
static int node_count = 0;

#define STATUS_OK       0000
#define STATUS_WARNING  0001
#define STATUS_ERROR    0002
#define STATUS_CRITICAL 0377

static Node* find_node_by_id(int id) {
    int i;
    for (i = 0; i < node_count; i++) {
        if (node_storage[i].id == id) {
            return &node_storage[i];
        }
    }
    return NULL;
}

static int add_node(int id, int parent_id, double value) {
    if (node_count >= MAX_NODES) {
        return STATUS_ERROR;
    }

    node_storage[node_count].id = id;
    node_storage[node_count].parent_id = parent_id;
    node_storage[node_count].value = value;

    node_storage[node_count].data[0] = 0100;
    node_storage[node_count].data[1] = 0200;
    node_storage[node_count].data[2] = 0300;
    node_storage[node_count].data[3] = 0400;

    node_count++;
    return STATUS_OK;
}

static int process_backward(int *array, size_t size, int start_offset) {
    int sum = 0;
    int *ptr;
    int *start;

    ptr = array + size;
    start = array + start_offset;

    while (ptr > start) {
        ptr--;
        sum += *ptr;
    }

    return sum;
}

static int compute_size_metric(const char *str) {
    size_t len = strlen(str);
    int metric;

    metric = (int)len;

    metric = metric * 2 + 010;

    return metric;
}

static int safe_double_to_int(double value) {
    if (value > 2147483647.0) {
        value = 2147483647.0;
    }
    if (value < -2147483648.0) {
        value = -2147483648.0;
    }

    return (int)value;
}

int jumpnode(int operation_mode, int node_id, int depth, int flags) {
    Node *current_node;
    Node *parent_node;
    int result = 0;
    int i;
    double accumulated_value;
    int temp_array[20];
    size_t array_size;
    char buffer[50];

    switch (operation_mode) {
        case 0001:
            current_node = find_node_by_id(node_id);
            if (current_node == NULL) {
                return STATUS_ERROR | 0020;
            }

            accumulated_value = current_node->value;

            for (i = 0; i < depth && current_node->parent_id != -1; i++) {
                parent_node = find_node_by_id(current_node->parent_id);
                if (parent_node == NULL) {
                    break;
                }

                accumulated_value += parent_node->value * 1.5;
                current_node = parent_node;
            }

            result = safe_double_to_int(accumulated_value);
            break;

        case 0002:
            current_node = find_node_by_id(node_id);
            if (current_node == NULL) {
                return STATUS_ERROR | 0040;
            }

            for (i = 0; i < 4; i++) {
                temp_array[i] = current_node->data[i];
            }

            for (i = 4; i < 020; i++) {
                temp_array[i] = i * 0007;
            }

            array_size = 020;

            result = process_backward(temp_array, array_size, depth);

            result += (int)array_size * flags;
            break;

        case 0003:
            sprintf(buffer, "Node_%d_Depth_%d", node_id, depth);

            result = compute_size_metric(buffer);

            result += (flags & 0177); /* Mask with octal 0177 */
            break;

        case 0004:
            current_node = find_node_by_id(node_id);
            if (current_node == NULL) {
                return STATUS_ERROR | 0100;
            }

            accumulated_value = 0.0;
            for (i = 0; i < 4; i++) {
                accumulated_value += sqrt((double)current_node->data[i]) * 2.718281828;
            }

            accumulated_value *= (1.0 + depth * 0.1);

            result = safe_double_to_int(accumulated_value);

            if (node_count > 2) {
                Node *end_ptr = &node_storage[node_count];
                Node *iter = end_ptr;
                int backward_sum = 0;

                for (i = 0; i < 3 && iter > node_storage; i++) {
                    iter--;
                    backward_sum += safe_double_to_int(iter->value);
                }

                result += backward_sum;
            }
            break;

        default:
            result = STATUS_ERROR | 0200;
            break;
    }

    return result;
}

static void initialize_test_data(void) {
    node_count = 0;

    add_node(1, -1, 100.5);
    add_node(2, 1, 50.25);
    add_node(3, 1, 75.75);
    add_node(4, 2, 25.125);
    add_node(5, 2, 30.875);
    add_node(6, 3, 40.0625);
    add_node(7, 4, 12.5);
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
    returns = jumpnode(a, b, c, d);
    printf("%d\n", returns);
    return 0;
}
