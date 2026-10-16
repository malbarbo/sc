#include <stdlib.h>
#include <string.h>

int main(void) {
    char *nome = malloc(10);
    strcpy(nome, "ana");
    free(nome);
    free(nome);
}
