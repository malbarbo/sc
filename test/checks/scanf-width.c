#include <stdio.h>
#include <stdlib.h>

int main(void) {
    char *sigla = malloc(4);
    if (scanf("%9s", sigla) == 1) {
        printf("%s\n", sigla);
    }
    free(sigla);
}
