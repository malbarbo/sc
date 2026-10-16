#include <stdio.h>
#include <stdlib.h>

struct no {
    int valor;
    struct no *prox;
};

struct no *lista = NULL;

int main(void) {
    for (int i = 1; i <= 3; i++) {
        struct no *n = malloc(sizeof(struct no));
        n->valor = i;
        n->prox = lista;
        lista = n;
    }
    printf("%d\n", lista->valor);
}
