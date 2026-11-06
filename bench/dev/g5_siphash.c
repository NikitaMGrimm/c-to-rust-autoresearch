#include <stddef.h>
#include <stdio.h>

void siphash(int init);


static size_t stbds_siphash_bytes(void *p, size_t len, size_t seed) {
  unsigned char *d = (unsigned char *)p;
  size_t i, j;
  size_t v0, v1, v2, v3, data;
  v0 = ((((size_t)0x736f6d65 << 16) << 16) + 0x70736575) ^ seed;
  v1 = ((((size_t)0x646f7261 << 16) << 16) + 0x6e646f6d) ^ ~seed;
  v2 = ((((size_t)0x6c796765 << 16) << 16) + 0x6e657261) ^ seed;
  v3 = ((((size_t)0x74656462 << 16) << 16) + 0x79746573) ^ ~seed;
  v0 ^= 0x0706050403020100ull ^ seed;
  v1 ^= 0x0f0e0d0c0b0a0908ull ^ ~seed;
  v2 ^= 0x0706050403020100ull ^ seed;
  v3 ^= 0x0f0e0d0c0b0a0908ull ^ ~seed;
  for (i = 0; i + sizeof(size_t) <= len;
       i += sizeof(size_t), d += sizeof(size_t)) {
    data = d[0] | (d[1] << 8) | (d[2] << 16) | (d[3] << 24);
    data |= (size_t)(d[4] | (d[5] << 8) | (d[6] << 16) | (d[7] << 24))
            << 16 << 16;
    v3 ^= data;
    for (j = 0; j < 2; ++j)
      do {
        v0 += v1;
        v1 = (((v1) << (13)) | ((v1) >> (((sizeof(size_t)) * 8) - (13))));
        v1 ^= v0;
        v0 =
            (((v0) << (((sizeof(size_t)) * 8) / 2)) |
             ((v0) >> (((sizeof(size_t)) * 8) - (((sizeof(size_t)) * 8) / 2))));
        v2 += v3;
        v3 = (((v3) << (16)) | ((v3) >> (((sizeof(size_t)) * 8) - (16))));
        v3 ^= v2;
        v2 += v1;
        v1 = (((v1) << (17)) | ((v1) >> (((sizeof(size_t)) * 8) - (17))));
        v1 ^= v2;
        v2 =
            (((v2) << (((sizeof(size_t)) * 8) / 2)) |
             ((v2) >> (((sizeof(size_t)) * 8) - (((sizeof(size_t)) * 8) / 2))));
        v0 += v3;
        v3 = (((v3) << (21)) | ((v3) >> (((sizeof(size_t)) * 8) - (21))));
        v3 ^= v0;
      } while (0);
    v0 ^= data;
  }
  data = len << (((sizeof(size_t)) * 8) - 8);
  switch (len - i) {
  case 7:
    data |= ((size_t)d[6] << 24) << 24;
  case 6:
    data |= ((size_t)d[5] << 20) << 20;
  case 5:
    data |= ((size_t)d[4] << 16) << 16;
  case 4:
    data |= (d[3] << 24);
  case 3:
    data |= (d[2] << 16);
  case 2:
    data |= (d[1] << 8);
  case 1:
    data |= d[0];
  case 0:
    break;
  }
  v3 ^= data;
  for (j = 0; j < 2; ++j)
    do {
      v0 += v1;
      v1 = (((v1) << (13)) | ((v1) >> (((sizeof(size_t)) * 8) - (13))));
      v1 ^= v0;
      v0 = (((v0) << (((sizeof(size_t)) * 8) / 2)) |
            ((v0) >> (((sizeof(size_t)) * 8) - (((sizeof(size_t)) * 8) / 2))));
      v2 += v3;
      v3 = (((v3) << (16)) | ((v3) >> (((sizeof(size_t)) * 8) - (16))));
      v3 ^= v2;
      v2 += v1;
      v1 = (((v1) << (17)) | ((v1) >> (((sizeof(size_t)) * 8) - (17))));
      v1 ^= v2;
      v2 = (((v2) << (((sizeof(size_t)) * 8) / 2)) |
            ((v2) >> (((sizeof(size_t)) * 8) - (((sizeof(size_t)) * 8) / 2))));
      v0 += v3;
      v3 = (((v3) << (21)) | ((v3) >> (((sizeof(size_t)) * 8) - (21))));
      v3 ^= v0;
    } while (0);
  v0 ^= data;
  v2 ^= 0xff;
  for (j = 0; j < 4; ++j)
    do {
      v0 += v1;
      v1 = (((v1) << (13)) | ((v1) >> (((sizeof(size_t)) * 8) - (13))));
      v1 ^= v0;
      v0 = (((v0) << (((sizeof(size_t)) * 8) / 2)) |
            ((v0) >> (((sizeof(size_t)) * 8) - (((sizeof(size_t)) * 8) / 2))));
      v2 += v3;
      v3 = (((v3) << (16)) | ((v3) >> (((sizeof(size_t)) * 8) - (16))));
      v3 ^= v2;
      v2 += v1;
      v1 = (((v1) << (17)) | ((v1) >> (((sizeof(size_t)) * 8) - (17))));
      v1 ^= v2;
      v2 = (((v2) << (((sizeof(size_t)) * 8) / 2)) |
            ((v2) >> (((sizeof(size_t)) * 8) - (((sizeof(size_t)) * 8) / 2))));
      v0 += v3;
      v3 = (((v3) << (21)) | ((v3) >> (((sizeof(size_t)) * 8) - (21))));
      v3 ^= v0;
    } while (0);
  return v0 ^ v1 ^ v2 ^ v3;
}

size_t stbds_hash_bytes(void *p, size_t len, size_t seed) {
  return stbds_siphash_bytes(p, len, seed);
}

void siphash(int init) {
  unsigned char mem[64];
  int i,j;
  int z = init;
  for (i=0; i < 64; ++i,z++) mem[i] = z;
  for (i=0; i < 64; ++i) {
    size_t hash = stbds_hash_bytes(mem, i, 0);
    printf("  { ");
    for (j=0; j < 8; ++j)
      printf("0x%02x, ", (unsigned char) ((hash >> (j*8)) & 255));
    printf(" },\n");
  }
}

/* Added by the benchmark intake in place of TRACTOR's test runner. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 2) {
        return 2;
    }
    int init = (int)strtoll(argv[1], NULL, 10);
    siphash(init);
    return 0;
}
