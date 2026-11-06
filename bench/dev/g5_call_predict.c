int call_predict(int pfcn);


typedef unsigned short btac1c_u16;
typedef signed short btac1c_s16;
typedef unsigned char btac1c_byte;
typedef struct btac1c_idxstate_s btac1c_idxstate;
struct btac1c_idxstate_s {
    btac1c_u16 idx;
    btac1c_s16 lpred;
    btac1c_s16 rpred;
    btac1c_byte tag;
    btac1c_byte bcfcn;
    btac1c_byte bsfcn;
    btac1c_byte usefx;
    btac1c_s16 firfx[4][8];
};

static int BTAC1C2_PredictSample(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    int pred, p0, p1;
    int i;
    i = idx;
    switch (pfcn) {
    case 0:
        pred = psamp[(i - 1) & 7];
        break;
    case 1:
        pred = 2 * psamp[(i - 1) & 7] - psamp[(i - 2) & 7];
        break;
    case 2:
        pred = (3 * psamp[(i - 1) & 7] - psamp[(i - 2) & 7]) >> 1;
        break;
    case 3:
        pred = (5 * psamp[(i - 1) & 7] - psamp[(i - 2) & 7]) >> 2;
        break;
    case 4:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7]);
        p1 = (psamp[(i - 2) & 7] + psamp[(i - 3) & 7]);
        pred = p0 - (p1 >> 1);
        break;
    case 5:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7]);
        p1 = (psamp[(i - 2) & 7] + psamp[(i - 3) & 7]);
        pred = (3 * p0 - p1) >> 2;
        break;
    case 6:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7]);
        p1 = (psamp[(i - 2) & 7] + psamp[(i - 3) & 7]);
        pred = (5 * p0 - p1) >> 3;
        break;
    case 7:
        pred = (18 * psamp[(i - 1) & 7] - 4 * psamp[(i - 2) & 7] +
                3 * psamp[(i - 3) & 7] - 2 * psamp[(i - 4) & 7] +
                1 * psamp[(i - 5) & 7]) /
               16;
        break;
    case 8:
        pred = (72 * psamp[(i - 1) & 7] - 16 * psamp[(i - 2) & 7] +
                12 * psamp[(i - 3) & 7] - 8 * psamp[(i - 4) & 7] +
                5 * psamp[(i - 5) & 7] - 3 * psamp[(i - 6) & 7] +
                3 * psamp[(i - 7) & 7] - 1 * psamp[(i - 8) & 7]) /
               64;
        break;
    case 9:
        pred = (76 * psamp[(i - 1) & 7] - 17 * psamp[(i - 2) & 7] +
                10 * psamp[(i - 3) & 7] - 7 * psamp[(i - 4) & 7] +
                5 * psamp[(i - 5) & 7] - 4 * psamp[(i - 6) & 7] +
                4 * psamp[(i - 7) & 7] - 3 * psamp[(i - 8) & 7]) /
               64;
        break;
    case 10:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7] + psamp[(i - 3) & 7] +
              psamp[(i - 4) & 7]);
        p1 = (psamp[(i - 5) & 7] + psamp[(i - 6) & 7] + psamp[(i - 7) & 7] +
              psamp[(i - 8) & 7]);
        pred = (5 * p0 - p1) >> 4;
        break;
    case 11:
        p0 = (psamp[(i - 1) & 7] + psamp[(i - 2) & 7] + psamp[(i - 3) & 7] +
              psamp[(i - 4) & 7]);
        p1 = (psamp[(i - 5) & 7] + psamp[(i - 6) & 7] + psamp[(i - 7) & 7] +
              psamp[(i - 8) & 7]);
        pred = (p0 + p1) >> 3;
        break;
    case 12:
    case 13:
    case 14:
    case 15:
        pred = (ridx->firfx[pfcn - 12][0] * psamp[(i - 1) & 7] +
                ridx->firfx[pfcn - 12][1] * psamp[(i - 2) & 7] +
                ridx->firfx[pfcn - 12][2] * psamp[(i - 3) & 7] +
                ridx->firfx[pfcn - 12][3] * psamp[(i - 4) & 7] +
                ridx->firfx[pfcn - 12][4] * psamp[(i - 5) & 7] +
                ridx->firfx[pfcn - 12][5] * psamp[(i - 6) & 7] +
                ridx->firfx[pfcn - 12][6] * psamp[(i - 7) & 7] +
                ridx->firfx[pfcn - 12][7] * psamp[(i - 8) & 7]) /
               256;
        break;
    default:
        pred = 0;
        break;
    }
    return (pred);
}

static int BTAC1C2_PredictSample_Pfn0(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    return (psamp[(idx - 1) & 7]);
}

static int BTAC1C2_PredictSample_Pfn1(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    return (2 * psamp[(idx - 1) & 7] - psamp[(idx - 2) & 7]);
}

static int BTAC1C2_PredictSample_Pfn2(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    return ((3 * psamp[(idx - 1) & 7] - psamp[(idx - 2) & 7]) >> 1);
}

static int BTAC1C2_PredictSample_Pfn3(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    return ((5 * psamp[(idx - 1) & 7] - psamp[(idx - 2) & 7]) >> 2);
}

static int BTAC1C2_PredictSample_Pfn4(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    int p0, p1;
    p0 = (psamp[(idx - 1) & 7] + psamp[(idx - 2) & 7]);
    p1 = (psamp[(idx - 2) & 7] + psamp[(idx - 3) & 7]);
    return (p0 - (p1 >> 1));
}

static int BTAC1C2_PredictSample_Pfn5(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    int p0, p1;
    p0 = (psamp[(idx - 1) & 7] + psamp[(idx - 2) & 7]);
    p1 = (psamp[(idx - 2) & 7] + psamp[(idx - 3) & 7]);
    return ((3 * p0 - p1) >> 2);
}

static int BTAC1C2_PredictSample_Pfn6(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    int p0, p1;
    p0 = (psamp[(idx - 1) & 7] + psamp[(idx - 2) & 7]);
    p1 = (psamp[(idx - 2) & 7] + psamp[(idx - 3) & 7]);
    return ((5 * p0 - p1) >> 3);
}

static int BTAC1C2_PredictSample_Pfn7(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    return ((18 * psamp[(idx - 1) & 7] - 4 * psamp[(idx - 2) & 7] +
             3 * psamp[(idx - 3) & 7] - 2 * psamp[(idx - 4) & 7] +
             1 * psamp[(idx - 5) & 7]) /
            16);
}

static int BTAC1C2_PredictSample_Pfn8(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    return ((72 * psamp[(idx - 1) & 7] - 16 * psamp[(idx - 2) & 7] +
             12 * psamp[(idx - 3) & 7] - 8 * psamp[(idx - 4) & 7] +
             5 * psamp[(idx - 5) & 7] - 3 * psamp[(idx - 6) & 7] +
             3 * psamp[(idx - 7) & 7] - 1 * psamp[(idx - 8) & 7]) /
            64);
}

static int BTAC1C2_PredictSample_Pfn9(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    return ((76 * psamp[(idx - 1) & 7] - 17 * psamp[(idx - 2) & 7] +
             10 * psamp[(idx - 3) & 7] - 7 * psamp[(idx - 4) & 7] +
             5 * psamp[(idx - 5) & 7] - 4 * psamp[(idx - 6) & 7] +
             4 * psamp[(idx - 7) & 7] - 3 * psamp[(idx - 8) & 7]) /
            64);
}

static int BTAC1C2_PredictSample_Pfn10(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    int p0, p1;
    p0 = (psamp[(idx - 1) & 7] + psamp[(idx - 2) & 7] + psamp[(idx - 3) & 7] +
          psamp[(idx - 4) & 7]);
    p1 = (psamp[(idx - 5) & 7] + psamp[(idx - 6) & 7] + psamp[(idx - 7) & 7] +
          psamp[(idx - 8) & 7]);
    return ((5 * p0 - p1) >> 3);
}

static int BTAC1C2_PredictSample_Pfn11(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx) {
    int p0, p1;
    p0 = (psamp[(idx - 1) & 7] + psamp[(idx - 2) & 7] + psamp[(idx - 3) & 7] +
          psamp[(idx - 4) & 7]);
    p1 = (psamp[(idx - 5) & 7] + psamp[(idx - 6) & 7] + psamp[(idx - 7) & 7] +
          psamp[(idx - 8) & 7]);
    return ((p0 + p1) >> 1);
}

static void *BTAC1C2_GetPredictFunc(int pfcn) {
    void *fcn;
    switch (pfcn) {
    case 0:
        fcn = (void *)BTAC1C2_PredictSample_Pfn0;
        break;
    case 1:
        fcn = (void *)BTAC1C2_PredictSample_Pfn1;
        break;
    case 2:
        fcn = (void *)BTAC1C2_PredictSample_Pfn2;
        break;
    case 3:
        fcn = (void *)BTAC1C2_PredictSample_Pfn3;
        break;
    case 4:
        fcn = (void *)BTAC1C2_PredictSample_Pfn4;
        break;
    case 5:
        fcn = (void *)BTAC1C2_PredictSample_Pfn5;
        break;
    case 6:
        fcn = (void *)BTAC1C2_PredictSample_Pfn6;
        break;
    case 7:
        fcn = (void *)BTAC1C2_PredictSample_Pfn7;
        break;
    case 8:
        fcn = (void *)BTAC1C2_PredictSample_Pfn8;
        break;
    case 9:
        fcn = (void *)BTAC1C2_PredictSample_Pfn9;
        break;
    case 10:
        fcn = (void *)BTAC1C2_PredictSample_Pfn10;
        break;
    case 11:
        fcn = (void *)BTAC1C2_PredictSample_Pfn11;
        break;
    default:
        fcn = (void *)BTAC1C2_PredictSample;
        break;
    }
    return (fcn);
}

int call_predict(int pfcn) {
    int result = 0;
    void *fcn = BTAC1C2_GetPredictFunc(pfcn);
    switch (pfcn) {
    case 0:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn0;
        break;
    case 1:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn1;
        break;
    case 2:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn2;
        break;
    case 3:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn3;
        break;
    case 4:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn4;
        break;
    case 5:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn5;
        break;
    case 6:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn6;
        break;
    case 7:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn7;
        break;
    case 8:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn8;
        break;
    case 9:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn9;
        break;
    case 10:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn10;
        break;
    case 11:
        result = fcn == (void *)BTAC1C2_PredictSample_Pfn11;
        break;
    default:
	break;
    }
    return result;
}

/* Added by the benchmark intake in place of TRACTOR's test runner. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 3) {
        return 2;
    }
    int pfcn = (int)strtoll(argv[1], NULL, 10);
    int returns = (int)strtoll(argv[2], NULL, 10);
    returns = call_predict(pfcn);
    printf("%d\n", returns);
    return 0;
}
