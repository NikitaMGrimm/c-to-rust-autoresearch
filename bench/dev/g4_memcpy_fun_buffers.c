// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdbool.h>

// ==================== Data Structures ====================

typedef struct {
    uint8_t data[256];
    size_t length;
    uint32_t checksum;
} buffer_t;

typedef struct {
    buffer_t *buffers;
    int count;
    int capacity;
} buffer_array_t;

typedef enum {
    OP_COPY = 0,
    OP_REVERSE = 1,
    OP_MERGE = 2,
    OP_SPLIT = 3,
    OP_INTERLEAVE = 4,
    OP_ROTATE = 5,
    OP_CHECKSUM = 6
} operation_t;

// ==================== Helper Functions ====================

// Calculate simple checksum
uint32_t calculate_checksum(const uint8_t *data, size_t length) {
    uint32_t sum = 0;
    for (size_t i = 0; i < length; i++) {
        sum = (sum << 3) ^ data[i];
    }
    return sum;
}

// Validate buffer integrity
bool validate_buffer(const buffer_t *buf) {
    if (buf == NULL) {
        fprintf(stderr, "Error: NULL buffer\n");
        return false;
    }
    if (buf->length > 256) {
        fprintf(stderr, "Error: Buffer length %zu exceeds maximum 256\n", buf->length);
        return false;
    }
    uint32_t expected = calculate_checksum(buf->data, buf->length);
    if (buf->checksum != expected) {
        fprintf(stderr, "Warning: Checksum mismatch. Expected %u, got %u\n", 
                expected, buf->checksum);
    }
    return true;
}

// Initialize buffer array
buffer_array_t* init_buffer_array(int initial_capacity) {
    if (initial_capacity <= 0) {
        fprintf(stderr, "Error: Invalid capacity %d\n", initial_capacity);
        return NULL;
    }
    
    buffer_array_t *arr = malloc(sizeof(buffer_array_t));
    if (!arr) {
        fprintf(stderr, "Error: Failed to allocate buffer array\n");
        return NULL;
    }
    
    arr->buffers = malloc(sizeof(buffer_t) * initial_capacity);
    if (!arr->buffers) {
        fprintf(stderr, "Error: Failed to allocate buffer storage\n");
        free(arr);
        return NULL;
    }
    
    arr->count = 0;
    arr->capacity = initial_capacity;
    return arr;
}

// Free buffer array
void free_buffer_array(buffer_array_t *arr) {
    if (arr) {
        free(arr->buffers);
        free(arr);
    }
}

// ==================== Core Buffer Operations ====================

// Simple copy operation with memcpy
int buffer_copy(const buffer_t *src, buffer_t *dst) {
    if (!src || !dst) {
        fprintf(stderr, "Error: NULL pointer in buffer_copy\n");
        return -1;
    }
    
    if (!validate_buffer(src)) {
        return -1;
    }
    
    // Use memcpy for data transfer
    memcpy(dst->data, src->data, src->length);
    dst->length = src->length;
    dst->checksum = calculate_checksum(dst->data, dst->length);
    
    return 0;
}

// Reverse buffer contents
int buffer_reverse(buffer_t *buf) {
    if (!buf) {
        fprintf(stderr, "Error: NULL buffer in reverse\n");
        return -1;
    }
    
    if (buf->length == 0) {
        return 0;  // Nothing to reverse
    }
    
    uint8_t temp[256];
    memcpy(temp, buf->data, buf->length);
    
    for (size_t i = 0; i < buf->length; i++) {
        buf->data[i] = temp[buf->length - 1 - i];
    }
    
    buf->checksum = calculate_checksum(buf->data, buf->length);
    return 0;
}

// Merge two buffers into destination
int buffer_merge(const buffer_t *src1, const buffer_t *src2, buffer_t *dst) {
    if (!src1 || !src2 || !dst) {
        fprintf(stderr, "Error: NULL pointer in buffer_merge\n");
        return -1;
    }
    
    if (src1->length + src2->length > 256) {
        fprintf(stderr, "Error: Merged length %zu exceeds maximum\n", 
                src1->length + src2->length);
        return -1;
    }
    
    // Copy first buffer
    memcpy(dst->data, src1->data, src1->length);
    // Copy second buffer after first
    memcpy(dst->data + src1->length, src2->data, src2->length);
    
    dst->length = src1->length + src2->length;
    dst->checksum = calculate_checksum(dst->data, dst->length);
    
    return 0;
}

// Split buffer at position into two buffers
int buffer_split(const buffer_t *src, size_t split_pos, 
                 buffer_t *dst1, buffer_t *dst2) {
    if (!src || !dst1 || !dst2) {
        fprintf(stderr, "Error: NULL pointer in buffer_split\n");
        return -1;
    }
    
    if (split_pos > src->length) {
        fprintf(stderr, "Error: Split position %zu exceeds length %zu\n", 
                split_pos, src->length);
        return -1;
    }
    
    // Copy first part
    if (split_pos > 0) {
        memcpy(dst1->data, src->data, split_pos);
    }
    dst1->length = split_pos;
    dst1->checksum = calculate_checksum(dst1->data, dst1->length);
    
    // Copy second part
    size_t remaining = src->length - split_pos;
    if (remaining > 0) {
        memcpy(dst2->data, src->data + split_pos, remaining);
    }
    dst2->length = remaining;
    dst2->checksum = calculate_checksum(dst2->data, dst2->length);
    
    return 0;
}

// Interleave two buffers (alternating bytes)
int buffer_interleave(const buffer_t *src1, const buffer_t *src2, buffer_t *dst) {
    if (!src1 || !src2 || !dst) {
        fprintf(stderr, "Error: NULL pointer in buffer_interleave\n");
        return -1;
    }
    
    size_t max_len = src1->length > src2->length ? src1->length : src2->length;
    if (src1->length + src2->length > 256) {
        fprintf(stderr, "Error: Interleaved length exceeds maximum\n");
        return -1;
    }
    
    size_t dst_pos = 0;
    for (size_t i = 0; i < max_len; i++) {
        if (i < src1->length) {
            memcpy(&dst->data[dst_pos++], &src1->data[i], 1);
        }
        if (i < src2->length) {
            memcpy(&dst->data[dst_pos++], &src2->data[i], 1);
        }
    }
    
    dst->length = dst_pos;
    dst->checksum = calculate_checksum(dst->data, dst->length);
    
    return 0;
}

// Rotate buffer left by n positions
int buffer_rotate(buffer_t *buf, int positions) {
    if (!buf) {
        fprintf(stderr, "Error: NULL buffer in rotate\n");
        return -1;
    }
    
    if (buf->length == 0 || positions == 0) {
        return 0;  // Nothing to rotate
    }
    
    // Normalize positions to valid range
    positions = positions % (int)buf->length;
    if (positions < 0) {
        positions += buf->length;
    }
    
    uint8_t temp[256];
    memcpy(temp, buf->data, buf->length);
    
    // Copy rotated portions
    memcpy(buf->data, temp + positions, buf->length - positions);
    memcpy(buf->data + (buf->length - positions), temp, positions);
    
    buf->checksum = calculate_checksum(buf->data, buf->length);
    
    return 0;
}

// Conditional copy based on pattern matching
int buffer_conditional_copy(const buffer_t *src, buffer_t *dst, 
                            uint8_t pattern, bool copy_matching) {
    if (!src || !dst) {
        fprintf(stderr, "Error: NULL pointer in conditional_copy\n");
        return -1;
    }
    
    size_t dst_pos = 0;
    for (size_t i = 0; i < src->length; i++) {
        bool matches = (src->data[i] == pattern);
        if (matches == copy_matching) {
            memcpy(&dst->data[dst_pos++], &src->data[i], 1);
        }
    }
    
    dst->length = dst_pos;
    dst->checksum = calculate_checksum(dst->data, dst->length);
    
    return 0;
}

// Copy with stride (every nth byte)
int buffer_copy_strided(const buffer_t *src, buffer_t *dst, int stride) {
    if (!src || !dst) {
        fprintf(stderr, "Error: NULL pointer in copy_strided\n");
        return -1;
    }
    
    if (stride <= 0) {
        fprintf(stderr, "Error: Invalid stride %d\n", stride);
        return -1;
    }
    
    size_t dst_pos = 0;
    for (size_t i = 0; i < src->length; i += stride) {
        memcpy(&dst->data[dst_pos++], &src->data[i], 1);
    }
    
    dst->length = dst_pos;
    dst->checksum = calculate_checksum(dst->data, dst->length);
    
    return 0;
}

// ==================== Complex Processing Functions ====================

// Process buffer array with operation
int process_buffer_array(buffer_array_t *arr, operation_t op, int param) {
    if (!arr || arr->count == 0) {
        fprintf(stderr, "Error: Invalid buffer array\n");
        return -1;
    }
    
    switch (op) {
        case OP_COPY:
            // Copy first buffer to all others
            for (int i = 1; i < arr->count; i++) {
                if (buffer_copy(&arr->buffers[0], &arr->buffers[i]) != 0) {
                    return -1;
                }
            }
            break;
            
        case OP_REVERSE:
            // Reverse all buffers
            for (int i = 0; i < arr->count; i++) {
                if (buffer_reverse(&arr->buffers[i]) != 0) {
                    return -1;
                }
            }
            break;
            
        case OP_MERGE:
            // Merge consecutive pairs
            if (arr->count < 2) {
                fprintf(stderr, "Error: Need at least 2 buffers for merge\n");
                return -1;
            }
            for (int i = 0; i < arr->count - 1; i += 2) {
                buffer_t merged;
                if (buffer_merge(&arr->buffers[i], &arr->buffers[i+1], &merged) != 0) {
                    return -1;
                }
                memcpy(&arr->buffers[i], &merged, sizeof(buffer_t));
            }
            break;
            
        case OP_ROTATE:
            // Rotate all buffers by param positions
            for (int i = 0; i < arr->count; i++) {
                if (buffer_rotate(&arr->buffers[i], param) != 0) {
                    return -1;
                }
            }
            break;
            
        case OP_CHECKSUM:
            // Verify all checksums
            for (int i = 0; i < arr->count; i++) {
                if (!validate_buffer(&arr->buffers[i])) {
                    return -1;
                }
            }
            break;
            
        default:
            fprintf(stderr, "Error: Unknown operation %d\n", op);
            return -1;
    }
    
    return 0;
}

// ==================== Input/Output Functions ====================

// Read buffer from stdin
int read_buffer(buffer_t *buf) {
    if (!buf) {
        fprintf(stderr, "Error: NULL buffer in read_buffer\n");
        return -1;
    }
    
    int length;
    if (scanf("%d", &length) != 1) {
        fprintf(stderr, "Error: Failed to read buffer length\n");
        return -1;
    }
    
    if (length < 0 || length > 256) {
        fprintf(stderr, "Error: Invalid buffer length %d\n", length);
        return -1;
    }
    
    buf->length = length;
    for (size_t i = 0; i < buf->length; i++) {
        int byte;
        if (scanf("%d", &byte) != 1) {
            fprintf(stderr, "Error: Failed to read byte %zu\n", i);
            return -1;
        }
        buf->data[i] = (uint8_t)byte;
    }
    
    buf->checksum = calculate_checksum(buf->data, buf->length);
    return 0;
}

// Write buffer to stdout
void write_buffer(const buffer_t *buf) {
    if (!buf) {
        fprintf(stderr, "Error: NULL buffer in write_buffer\n");
        return;
    }
    
    printf("%zu", buf->length);
    for (size_t i = 0; i < buf->length; i++) {
        printf(" %u", buf->data[i]);
    }
    printf("\n");
}

// ==================== Main Function ====================

int main(int argc, char *argv[]) {
    int operation;
    int buffer_count;
    
    // Read operation type
    if (scanf("%d", &operation) != 1) {
        fprintf(stderr, "Error: Failed to read operation\n");
        return 1;
    }
    
    // Read buffer count
    if (scanf("%d", &buffer_count) != 1) {
        fprintf(stderr, "Error: Failed to read buffer count\n");
        return 1;
    }
    
    if (buffer_count <= 0 || buffer_count > 100) {
        fprintf(stderr, "Error: Invalid buffer count %d\n", buffer_count);
        return 1;
    }
    
    // Allocate buffer array
    buffer_array_t *buffers = init_buffer_array(buffer_count);
    if (!buffers) {
        return 1;
    }
    
    // Read all buffers
    for (int i = 0; i < buffer_count; i++) {
        if (read_buffer(&buffers->buffers[i]) != 0) {
            free_buffer_array(buffers);
            return 1;
        }
        buffers->count++;
    }
    
    // Execute operation based on type
    int result = 0;
    switch (operation) {
        case OP_COPY:
            if (buffer_count >= 2) {
                buffer_t temp;
                result = buffer_copy(&buffers->buffers[0], &temp);
                if (result == 0) {
                    write_buffer(&temp);
                }
            } else {
                fprintf(stderr, "Error: Copy needs at least 2 buffers\n");
                result = -1;
            }
            break;
            
        case OP_REVERSE:
            for (int i = 0; i < buffer_count; i++) {
                result = buffer_reverse(&buffers->buffers[i]);
                if (result != 0) break;
                write_buffer(&buffers->buffers[i]);
            }
            break;
            
        case OP_MERGE:
            if (buffer_count >= 2) {
                buffer_t merged;
                result = buffer_merge(&buffers->buffers[0], &buffers->buffers[1], &merged);
                if (result == 0) {
                    write_buffer(&merged);
                }
            } else {
                fprintf(stderr, "Error: Merge needs at least 2 buffers\n");
                result = -1;
            }
            break;
            
        case OP_SPLIT: {
            if (buffer_count >= 1) {
                int split_pos;
                if (scanf("%d", &split_pos) != 1) {
                    fprintf(stderr, "Error: Failed to read split position\n");
                    result = -1;
                } else {
                    buffer_t part1, part2;
                    result = buffer_split(&buffers->buffers[0], split_pos, &part1, &part2);
                    if (result == 0) {
                        write_buffer(&part1);
                        write_buffer(&part2);
                    }
                }
            }
            break;
        }
            
        case OP_INTERLEAVE:
            if (buffer_count >= 2) {
                buffer_t interleaved;
                result = buffer_interleave(&buffers->buffers[0], &buffers->buffers[1], &interleaved);
                if (result == 0) {
                    write_buffer(&interleaved);
                }
            } else {
                fprintf(stderr, "Error: Interleave needs at least 2 buffers\n");
                result = -1;
            }
            break;
            
        case OP_ROTATE: {
            int positions;
            if (scanf("%d", &positions) != 1) {
                fprintf(stderr, "Error: Failed to read rotation amount\n");
                result = -1;
            } else {
                for (int i = 0; i < buffer_count; i++) {
                    result = buffer_rotate(&buffers->buffers[i], positions);
                    if (result != 0) break;
                    write_buffer(&buffers->buffers[i]);
                }
            }
            break;
        }
            
        case OP_CHECKSUM:
            for (int i = 0; i < buffer_count; i++) {
                printf("%u\n", buffers->buffers[i].checksum);
            }
            break;
            
        default:
            fprintf(stderr, "Error: Unknown operation %d\n", operation);
            result = -1;
            break;
    }
    
    free_buffer_array(buffers);
    return result != 0 ? 1 : 0;
}
