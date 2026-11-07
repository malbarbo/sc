#include <stdio.h>
#include <stdlib.h>

int main(void) {
    int *v = malloc(4 * sizeof(int));
    int *p = v;
    for (int i = 0; i < 4; i++) *p++ = i * i;
    free(p);
}
