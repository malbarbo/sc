#include <stdio.h>
#include <stdlib.h>

int main(void) {
    int n = 5;
    int *quadrados = malloc(n * sizeof(int));
    for (int i = 0; i <= n; i++) {
        quadrados[i] = i * i;
    }
    printf("%d\n", quadrados[n - 1]);
    free(quadrados);
}
