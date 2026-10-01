// entry 5: data from the heap and a function pointer of entry 3
#include <stdio.h>
extern char *s;
int f(int);
int main(void) { printf("%s, f(3) = %d\n", s, f(3)); return 3; }
