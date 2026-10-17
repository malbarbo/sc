#include <stdio.h>
//%%
int x = 1;
//%%
int f(void) { return x * 10; }
//%%
//%undo
//%%
int f(void) { return x * 100; }
//%%
//%undo
//%%
//%undo
//%%
int x = 2;
//%%
int f(void) { return x * 1000; }
//%%
int main(void) { printf("%d\n", f()); }
