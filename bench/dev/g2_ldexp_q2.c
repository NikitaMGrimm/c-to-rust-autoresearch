float ldexp_q2(float y, int exp_q2);

float ldexp_q2(float y, int exp_q2) {
    static const float g_expfrac[4] = {9.31322575e-10f, 7.83145814e-10f,
                                       6.58544508e-10f, 5.53767716e-10f};
    int e;
    do {
        e = ((30 * 4) > (exp_q2) ? (exp_q2) : (30 * 4));
        y *= g_expfrac[e & 3] * (1 << 30 >> (e >> 2));
    } while ((exp_q2 -= e) > 0);
    return y;
}

/* Added by the benchmark intake in place of TRACTOR's test runner. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 4) {
        return 2;
    }
    float y = strtof(argv[1], NULL);
    int exp_q2 = (int)strtoll(argv[2], NULL, 10);
    float returns = strtof(argv[3], NULL);
    returns = ldexp_q2(y, exp_q2);
    printf("%.9g\n", returns);
    return 0;
}
