#include <stdio.h>
#include <stdlib.h>

struct no {
    int valor;
    struct no *prox;
};

int main(void) {
    struct no *lista = NULL;
    printf("%d\n", lista->valor);
}
