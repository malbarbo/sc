#include <stdio.h>
#include <stdlib.h>

struct no {
    int valor;
    struct no *prox;
};

void libera(struct no *lista) {
    while (lista != NULL) {
        free(lista);
        lista = lista->prox;
    }
}

int main(void) {
    struct no *lista = NULL;
    for (int i = 0; i < 3; i++) {
        struct no *n = malloc(sizeof(struct no));
        n->valor = i;
        n->prox = lista;
        lista = n;
    }
    libera(lista);
    printf("ok\n");
}
