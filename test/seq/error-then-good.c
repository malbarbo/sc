#include <stdio.h>
//%%
int f(void) { return undeclared + 1; }
//%%
int f(void) { return 1; }
//%%
int g(void) { return f() + 1 }
//%%
int g(void) { return f() + 2; }
//%%
int main(void) { printf("%d\n", g()); }
