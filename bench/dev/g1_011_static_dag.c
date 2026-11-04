// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>

int
static_update(bool update, int new_value) {
  static int run = 0;
  if(update) {
    run = new_value;
  }
  return run;
}

void
path_mult(int update) {
  int run = static_update(false, 0);
  run = run * update;
  int _updated = static_update(true, run);
  return;
}

void
path_add(int update) {
  int run = static_update(false, 0);
  run = run + update;
  int _updated = static_update(true, run);
  return;
}

void
path_subtract(int update) {
  int run = static_update(false, 0);
  run = run - update;
  int _updated = static_update(true, run);
  return;
}

/*
  Carry through static state across multiple function calls 
 */
int
main(int argc, char **argv) {

  if (argc != 3) {
    printf("Error: should only be two (integer) arguments!\n");
    return 1;
  }

  char *end;
  int val = strtol(argv[1], &end, 10);
  if (end == argv[1]) {
    // end is set to start of string if nothing parsed
    printf("Error: first argument must be an integer!\n");
    return 1;
  }
  
  int iterations = strtol(argv[2], &end, 10);
  if (end == argv[2]) {
    // end is set to start of string if nothing parsed
    printf("Error: second argument must be an integer!\n");
    return 1;
  }
    
  for (int i = 0; i < iterations; i++) {
    path_add(val);
    printf("%d\n", static_update(false, 0));
    path_mult(val);
    printf("%d\n", static_update(false, 0));
    path_subtract(val);
    printf("%d\n", static_update(false, 0));
  }

  return 0;
}
