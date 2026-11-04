// © 2026 Massachusetts Institute of Technology
// MIT License

#include <errno.h>
#include <limits.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    int floors;
    int bedrooms;
    double bathrooms;
} house_t;

static void add_floor(house_t *house) {
    house->floors++;
}

static void add_bedrooms(house_t *house, int extra_bedrooms) {
    house->bedrooms += extra_bedrooms;
}

static void print_house(house_t *house) {
    printf("The house has %d floors, %d bedrooms, and %.1f bathrooms\n", house->floors, house->bedrooms, house->bathrooms);
}

void run(house_t *the_house, int extra_bedrooms) {
    print_house(the_house);
    add_floor(the_house);
    print_house(the_house);
    the_house->bathrooms += 1.0;
    print_house(the_house);
    add_bedrooms(the_house, extra_bedrooms);
    print_house(the_house);
}

static bool parse_val(const char *str, int *val) {
    errno = 0;
    char *endp = (char *)str;
    long tmp = strtol(str, &endp, 10);
    if (endp != str && errno == 0 && tmp >= INT_MIN && tmp <= INT_MAX) {
        *val = tmp;
        return true;
    } else {
        return false;
    }
}

int main() {
    char in[100] = "";
    fgets(in, sizeof(in), stdin);
    int x;
    if (parse_val(in, &x)) {
        house_t the_house = {.floors = 2, .bedrooms = 5, .bathrooms = 2.5};
        run(&the_house, x);
        run(&the_house, x);
    } else {
        printf("An error occurred\n");
    }
    return 0;
}