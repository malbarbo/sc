#include <stdio.h>
//%%
static int hidden = 3;
//%%
static int get(void);
//%%
int use(void) { return get() + hidden; }
//%%
static int get(void) { return hidden * 2; }
//%%
static int get(void) { return 0; }
//%%
int main(void) { printf("%d\n", use()); }
