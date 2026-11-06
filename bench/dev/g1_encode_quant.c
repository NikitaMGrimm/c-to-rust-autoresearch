int encode_quant(int uni, int step, int pred, int tgt, int tgt2, int lsbit);


int encode_quant(int uni, int step, int pred, int tgt, int tgt2, int lsbit) {
    int uni1, uni2;
    int diff, p0, p1, p2, p3, d0, d1, d2, d3;
    uni1 = uni + 1;
    uni2 = uni - 1;
    if ((uni ^ uni1) & (~7))
        uni1 = uni;
    if ((uni ^ uni2) & (~7))
        uni2 = uni;
    if (lsbit) {
        if (lsbit == 4) {
            uni &= ~1;
            uni1 &= ~1;
            uni2 &= ~1;
            uni |= (uni >> 1) & (uni >> 2) & 1;
            uni1 |= (uni1 >> 1) & (uni1 >> 2) & 1;
            uni2 |= (uni2 >> 1) & (uni2 >> 2) & 1;
        } else if (lsbit & 1) {
            uni |= 1;
            uni1 |= 1;
            uni2 |= 1;
        } else {
            uni &= ~1;
            uni1 &= ~1;
            uni2 &= ~1;
        }
    }
    diff = ((2 * (uni & 7) + 1) * step) / 8;
    if (uni & 8)
        diff = -diff;
    p0 = pred + diff;
    d0 = tgt - p0;
    d0 = d0 ^ (d0 >> 31);
    diff = ((2 * (uni1 & 7) + 1) * step) / 8;
    if (uni1 & 8)
        diff = -diff;
    p1 = pred + diff;
    d1 = tgt - p1;
    d1 = d1 ^ (d1 >> 31);
    diff = ((2 * (uni2 & 7) + 1) * step) / 8;
    if (uni2 & 8)
        diff = -diff;
    p2 = pred + diff;
    d2 = tgt - p2;
    d2 = d2 ^ (d2 >> 31);
    d3 = tgt2 - p0;
    d3 = d3 ^ (d3 >> 31);
    d0 += d3 >> 5;
    d3 = tgt2 - p1;
    d3 = d3 ^ (d3 >> 31);
    d1 += d3 >> 5;
    d3 = tgt2 - p2;
    d3 = d3 ^ (d3 >> 31);
    d2 += d3 >> 5;
    if (d1 < d0)
        uni = uni1;
    if (d2 < d0)
        uni = uni2;
    return (uni);
}

/* Added by the benchmark intake in place of TRACTOR's test runner. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 8) {
        return 2;
    }
    int uni = (int)strtoll(argv[1], NULL, 10);
    int step = (int)strtoll(argv[2], NULL, 10);
    int pred = (int)strtoll(argv[3], NULL, 10);
    int tgt = (int)strtoll(argv[4], NULL, 10);
    int tgt2 = (int)strtoll(argv[5], NULL, 10);
    int lsbit = (int)strtoll(argv[6], NULL, 10);
    int returns = (int)strtoll(argv[7], NULL, 10);
    returns = encode_quant(uni, step, pred, tgt, tgt2, lsbit);
    printf("%d\n", returns);
    return 0;
}
