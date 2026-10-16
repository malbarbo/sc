#include <stdio.h>
#include <stdlib.h>

struct no {
    int valor;
    struct no *prox;
};

struct no *insere(struct no *lista, int valor) {
    struct no *n = malloc(sizeof(struct no));
    n->valor = valor;
    n->prox = lista;
    return n;
}

int soma(int n) {
    struct no *lista = NULL;
    for (int i = 1; i <= n; i++) {
        lista = insere(lista, i);
    }
    int s = 0;
    for (struct no *p = lista; p != NULL; p = p->prox) {
        s += p->valor;
    }
    return s;
}

int main(void) {
    printf("%d\n", soma(3));
}
