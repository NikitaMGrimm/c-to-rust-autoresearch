// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

typedef struct {
    int id;
    char name[32];
    uint8_t flags;
} DataBlock;

typedef struct {
    int *data;
    size_t size;
} MemoryBlock;

DataBlock create_block(int id, const char *name, uint8_t flags) {
    DataBlock block;
    block.id = id;
    strcpy(block.name, name);
    block.flags = flags;
    return block;
}

MemoryBlock* allocate_block(size_t count, int init_value) {
    MemoryBlock *mb = (MemoryBlock*)malloc(sizeof(MemoryBlock));
    if (!mb) return NULL;

    mb->data = (int*)calloc(count, sizeof(int));
    if (!mb->data) {
        free(mb);
        return NULL;
    }

    mb->size = count;

    for (size_t i = 0; i < count; i++) {
        mb->data[i] = init_value + i;
    }

    return mb;
}

void free_block(MemoryBlock *mb) {
    if (mb) {
        if (mb->data) free(mb->data);
        free(mb);
    }
}

int compute_hash(MemoryBlock *mb1, MemoryBlock *mb2) {
    int hash = 0;

    if (mb1->data < mb2->data) {
        hash += 100;
    } else if (mb1->data > mb2->data) {
        hash += 200;
    }

    if (mb1 < mb2) {
        hash += 10;
    } else if (mb1 > mb2) {
        hash += 20;
    }

    return hash;
}

int betagamma(int param1, int param2, int param3, int param4) {
    int result = 0;

    DataBlock blocks[] = {
        {1, "Block_Alpha", 0b10101010},
        {2, "Block_Beta", 0b11001100},
        {3, "Block_Gamma", 0b11110000}
    };

    int num_blocks = sizeof(blocks) / sizeof(blocks[0]);

    for (int i = 0; i < num_blocks; i++) {
        DataBlock *current = &blocks[i];

        char temp_name[32];
        strcpy(temp_name, current->name);

        int flag_contribution = 0;
        if (current->flags & 0b00001111) {
            flag_contribution += param1;
        }
        if (current->flags & 0b11110000) {
            flag_contribution += param2;
        }
        if (current->flags & 0b10101010) {
            flag_contribution += param3;
        }
        if (current->flags & 0b01010101) {
            flag_contribution += param4;
        }

        result += flag_contribution * current->id;
    }

    size_t block_size = (param1 % 10) + 5;
    MemoryBlock *mem1 = allocate_block(block_size, param1);
    MemoryBlock *mem2 = allocate_block(block_size, param2);

    if (!mem1 || !mem2) {
        free_block(mem1);
        free_block(mem2);
        return -1;
    }

    int hash = compute_hash(mem1, mem2);
    result += hash;

    int sum1 = 0, sum2 = 0;
    for (size_t i = 0; i < mem1->size; i++) {
        sum1 += mem1->data[i];
    }
    for (size_t i = 0; i < mem2->size; i++) {
        sum2 += mem2->data[i];
    }

    result += (sum1 - sum2) / 10;

    DataBlock special = {99, "Special", 0b11111111};
    strcpy(special.name, "Modified");

    if (mem1->data != mem2->data) {
        result += special.id;
    }

    if (mem1->data > NULL && mem2->data > NULL) {
        result += special.flags;
    }

    free_block(mem1);
    free_block(mem2);

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
    returns = betagamma(a, b, c, d);
    printf("%d\n", returns);
    return 0;
}
