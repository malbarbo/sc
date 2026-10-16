#include <stdio.h>
#include <stdlib.h>
#include <string.h>

char *copia(const char *s) {
    char *c = malloc(strlen(s));
    strcpy(c, s);
    return c;
}

int main(void) {
    char *nome = copia("maria");
    printf("%s\n", nome);
    free(nome);
}
