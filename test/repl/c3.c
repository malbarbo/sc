// entry 3: redefines f; pf still points to the old one
#include <stdio.h>
extern int (*pf)(int);
int f(int a) { return a * 100; }
__attribute__((constructor)) static void run(void) { printf("f(2) = %d, pf(1) = %d\n", f(2), pf(1)); }
