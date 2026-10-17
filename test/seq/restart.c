#include <stdio.h>
//%%
static int counter = 5;
//%%
static int next(void) { return ++counter; }
//%%
//%restart
//%%
int twice(void) { return next() + next(); }
//%%
//%restart
//%%
//%undo
//%%
int twice(void) { return next() * next(); }
//%%
int main(void) { printf("%d %d\n", twice(), counter); }
