// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>
#include <stdlib.h>

int
static_sum(int update) {
  static int sum = 0;
  sum += update;
  return sum;
}

/*
  Maintain a running total using a static variable
 */
int
main(int argc, char **argv) {

  if (argc != 2) {
    printf("Error: should only be a single (integer) argument!\n");
    return 1;
  }

  char *end;
  int stride = strtol(argv[1], &end, 10);
  if (end == argv[1]) {
    // end is set to start of string if nothing parsed
    printf("Error: first argument must be an integer!\n");
    return 1;
  }
    
  for (int i = 0; i < 10; i++) {
    printf("%d\n", static_sum(i * stride));
  }

  return 0;
}
