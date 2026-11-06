int div_euclid(int v1, int v2);

int div_euclid(int v1, int v2) {
    if (v2 == 0) {
        return 0;
    }
    int q, r;
    if (v1 >= 0)
        if (v2 >= 0)
            return ((v1) / (v2));
        else if (v2 != (-0x7fffffff - 1))
            q = -((v1) / (-v2)), r = ((v1) % (-v2));
        else
            q = 0, r = v1;
    else if (v1 != (-0x7fffffff - 1))
        if (v2 >= 0)
            q = -((-v1) / (v2)), r = -((-v1) % (v2));
        else if (v2 != (-0x7fffffff - 1))
            q = ((-v1) / (-v2)), r = -((-v1) % (-v2));
        else
            q = 1, r = v1 - q * v2;
    else if (v2 >= 0)
        q = -((-(v1 + v2)) / (v2)) - 1, r = -((-(v1 + v2)) % (v2));
    else if (v2 != (-0x7fffffff - 1))
        q = ((-(v1 - v2)) / (-v2)) + 1, r = -((-(v1 - v2)) % (-v2));
    else
        q = 1, r = 0;
    if (r >= 0)
        return q;
    else
        return q + (v2 > 0 ? -1 : 1);
}

/* Added by the benchmark intake in place of TRACTOR's test runner. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 4) {
        return 2;
    }
    int v1 = (int)strtoll(argv[1], NULL, 10);
    int v2 = (int)strtoll(argv[2], NULL, 10);
    int returns = (int)strtoll(argv[3], NULL, 10);
    returns = div_euclid(v1, v2);
    printf("%d\n", returns);
    return 0;
}
