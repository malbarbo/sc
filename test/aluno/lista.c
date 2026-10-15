// lista encadeada: insere ordenado, remove, imprime e libera
#include <stdio.h>
#include <stdlib.h>

typedef struct No {
    int valor;
    struct No *prox;
} No;

No *insere(No *lista, int valor) {
    No *novo = malloc(sizeof(No));
    if (novo == NULL) {
        exit(1);
    }
    novo->valor = valor;
    if (lista == NULL || valor < lista->valor) {
        novo->prox = lista;
        return novo;
    }
    No *p = lista;
    while (p->prox != NULL && p->prox->valor < valor) {
        p = p->prox;
    }
    novo->prox = p->prox;
    p->prox = novo;
    return lista;
}

No *remove_valor(No *lista, int valor) {
    No *ant = NULL;
    for (No *p = lista; p != NULL; ant = p, p = p->prox) {
        if (p->valor == valor) {
            if (ant == NULL) {
                lista = p->prox;
            } else {
                ant->prox = p->prox;
            }
            free(p);
            break;
        }
    }
    return lista;
}

void imprime(const No *lista) {
    printf("[");
    for (const No *p = lista; p != NULL; p = p->prox) {
        printf("%d%s", p->valor, p->prox ? ", " : "");
    }
    printf("]\n");
}

int tamanho(const No *lista) {
    return lista == NULL ? 0 : 1 + tamanho(lista->prox);
}

void libera(No *lista) {
    while (lista != NULL) {
        No *prox = lista->prox;
        free(lista);
        lista = prox;
    }
}

int main(void) {
    No *lista = NULL;
    int valores[] = {5, 2, 8, 1, 9, 3};
    int n = sizeof(valores) / sizeof(valores[0]);
    for (int i = 0; i < n; i++) {
        lista = insere(lista, valores[i]);
    }
    imprime(lista);
    lista = remove_valor(lista, 8);
    lista = remove_valor(lista, 1);
    imprime(lista);
    printf("tamanho: %d\n", tamanho(lista));
    libera(lista);
    return 0;
}
