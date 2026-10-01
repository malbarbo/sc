// entry 4: redefines x; the old f still reads the old x
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
extern int (*pf)(int);
int f(int);
int x = 7;
char *s;
__attribute__((constructor)) static void run(void) {
  s = strdup("heap between entries");
  printf("x = %d, f(1) = %d, pf(1) = %d, pf == f: %d\n", x, f(1), pf(1), pf == f);
}
