// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_ENTRIES 10
#define NAME_LENGTH 32

typedef struct {
    int id;
    int value;
    char name[NAME_LENGTH];
} DataEntry;

static int lookup_table[4][3] = {
    {10, 20, 30},
    {40, 50, 60},
    {70, 80, 90},
    {100, 110, 120}
};

static DataEntry* find_entry(DataEntry* entries, int count, int target_id) {
    DataEntry* ptr = entries;
    DataEntry* end = entries + count;

    while (ptr < end) {
        if (ptr->id == target_id) {
            return ptr;
        }
        ptr++;
    }

    return NULL;
}

static int process_name(char* dest, const char* src, int max_len) {
    int len;

    if (dest == NULL || *dest == '\0') {
        return -1;
    }

    strcpy(dest, src);

    len = strlen(dest);
    return len;
}

static int calculate_lookup(int row, int col, int* result) {
    int temp;

    if ((temp = lookup_table[row][col])) {
        *result = temp * 2;
        return 1;
    }

    return 0;
}

static DataEntry* create_entries(int count, int base_id) {
    DataEntry* entries;
    int i;
    char temp_name[NAME_LENGTH];

    entries = (DataEntry*)malloc(count * sizeof(DataEntry));

    if (entries == NULL || count <= 0) {
        return NULL;
    }

    for (i = 0; i < count; i++) {
        entries[i].id = base_id + i;
        entries[i].value = (base_id + i) * 10;

        sprintf(temp_name, "Entry_%d", base_id + i);

        strcpy(entries[i].name, temp_name);
    }

    return entries;
}

static int modify_entries(DataEntry* entries, int count, int multiplier) {
    DataEntry* current;
    DataEntry* last;
    int total = 0;
    int temp_value;

    if (entries == NULL) {
        return -1;
    }

    current = entries;
    last = entries + count;

    while (current < last) {
        if ((temp_value = current->value)) {
            current->value = temp_value * multiplier;
            total += current->value;
        }
        current++;
    }

    return total;
}

int dataentry(int mode, int param1, int param2, int param3) {
    DataEntry* entries = NULL;
    DataEntry* found = NULL;
    int result = 0;
    int count;
    int lookup_result;
    char buffer[NAME_LENGTH];
    int i;

    buffer[0] = 'T';
    buffer[1] = '\0';

    switch (mode) {
        case 1:
            count = param1 > 0 ? param1 : 5;
            entries = create_entries(count, 100);

            if (entries == NULL || count == 0) {
                result = -1;
                break;
            }

            found = find_entry(entries, count, 100 + param2);

            if (found == NULL || found->id == 0) {
                result = -2;
            } else {
                result = found->value;

                strcpy(buffer, found->name);
            }

            free(entries);
            break;

        case 2:
            count = param1 > 0 ? param1 : 3;
            entries = create_entries(count, 200);

            if (entries == NULL) {
                result = -1;
                break;
            }

            if ((result = modify_entries(entries, count, param2))) {
                result += param3;
            }

            free(entries);
            break;

        case 3:
            if (param1 >= 0 && param1 < 4 && param2 >= 0 && param2 < 3) {
                if ((result = calculate_lookup(param1, param2, &lookup_result))) {
                    result = lookup_result + param3;
                }
            }
            break;

        default:
            strcpy(buffer, "Default");
            result = process_name(buffer, "TestName", NAME_LENGTH);

            if ((count = strlen(buffer))) {
                result = count * param1;
            }
            break;
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
    returns = dataentry(a, b, c, d);
    printf("%d\n", returns);
    return 0;
}
