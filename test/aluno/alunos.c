// structs: cadastro, média, ordenação com qsort e busca
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    char nome[30];
    double notas[3];
    double media;
} Aluno;

double media(const double notas[], int n) {
    double soma = 0;
    for (int i = 0; i < n; i++) {
        soma += notas[i];
    }
    return soma / n;
}

int compara(const void *a, const void *b) {
    const Aluno *x = a;
    const Aluno *y = b;
    if (x->media < y->media) return 1;
    if (x->media > y->media) return -1;
    return strcmp(x->nome, y->nome);
}

int busca(const Aluno v[], int n, const char *nome) {
    for (int i = 0; i < n; i++) {
        if (strcmp(v[i].nome, nome) == 0) {
            return i;
        }
    }
    return -1;
}

int main(void) {
    Aluno turma[] = {
        {"Bruno", {7.5, 8.0, 6.5}, 0},
        {"Ana", {9.0, 9.5, 10.0}, 0},
        {"Carla", {5.0, 6.0, 7.0}, 0},
    };
    int n = sizeof turma / sizeof turma[0];
    for (int i = 0; i < n; i++) {
        turma[i].media = media(turma[i].notas, 3);
    }
    qsort(turma, n, sizeof(Aluno), compara);
    for (int i = 0; i < n; i++) {
        printf("%-10s %5.2f %s\n", turma[i].nome, turma[i].media, turma[i].media >= 7 ? "aprovado" : "exame");
    }
    printf("Carla na posicao %d\n", busca(turma, n, "Carla"));
    return 0;
}
