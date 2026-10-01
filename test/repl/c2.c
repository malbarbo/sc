// entry 2: uses entry 1 and keeps a pointer to its f
#include <stdio.h>
extern int x;
int f(int);
int (*pf)(int) = f;
__attribute__((constructor)) static void run(void) { printf("f(1) = %d\n", f(1)); x = 20; }
