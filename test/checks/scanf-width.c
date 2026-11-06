#include <stdio.h>
#include <stdlib.h>

int main(void) {
    char *acronym = malloc(4);
    if (scanf("%9s", acronym) == 1) {
        printf("%s\n", acronym);
    }
    free(acronym);
}
