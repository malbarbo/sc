#include <stdio.h>
#include <stdlib.h>
#include <stdbool.h>
#define DEBUG 0
const bool verbose = false;
enum { MAX = 10 };
int f(int x) {
  if (DEBUG) printf("dbg\n");
  if (verbose) printf("v\n");
  if (MAX > 100) printf("large\n");
  switch (x) { case 1: return 1; break; default: exit(1); return 0; }
}
int g(void) { while (1) { int c = getchar(); if (c == EOF) return 0; } return -1; }
int h(void) { for (;;) { return 1; } }
int main(void) { bool debug = false; if (debug) puts("x"); return f(1) + g() + h(); }
