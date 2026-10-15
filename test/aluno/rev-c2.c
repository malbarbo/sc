#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <assert.h>
#include <time.h>

int n_global = 0;
int total;

typedef struct No { int valor; struct No *prox; } No;

No *insere(No *lista, int valor) {
    No *novo = malloc(sizeof(No));
    novo->valor = valor;
    novo->prox = lista;
    return novo;
}

void libera(No *lista) {
    while (lista != NULL) { No *t = lista; lista = lista->prox; free(t); }
}

int eh_primo(int n) {
    int lim = sqrt(n);
    for (int i = 2; i <= lim; i++) if (n % i == 0) return 0;
    return n > 1;
}

void palindromo(const char *s) {
    int i, j;
    for (i = 0, j = strlen(s) - 1; i < j; i++, j--)
        if (s[i] != s[j]) { printf("nao\n"); return; }
    printf("sim\n");
}

int menu(void) {
    int op;
    while (1) {
        printf("1-a 2-b 0-sair\n");
        if (scanf("%d", &op) != 1) return 0;
        switch (op) {
        case 0: return 0;
        case 1: printf("a\n"); break;
        case 2: printf("b\n"); break;
        default: printf("invalida\n");
        }
    }
    return 0;
}

int procura(int *v, int n, int x) {
    for (int i = 0; i < n; i++) {
        if (v[i] == x) return i;
        else continue;
    }
    return -1;
}

int calc(char op, int a, int b) {
    int r;
    switch (op) {
    case '+': r = a + b; break;
    case '-': r = a - b; break;
    case '*': r = a * b; break;
    default: printf("erro\n"); exit(1);
    }
    return r;
}

int maximo(int v[], int n) {
    int max;
    for (int i = 0; i < n; i++)
        if (i == 0 || v[i] > max) max = v[i];
    return max;
}

double grau_para_rad(double g) { return g * M_PI / 180.0; }

int conta(int n) {
    int total = 0;
    for (int i = 0; i < n; i++) total += i;
    return total;
}

void preenche(int n) {
    for (int i = 0; i < n; i++) {
        for (int i = 0; i < 2; i++) printf("x");
    }
}

int main(void) {
    No *l = NULL;
    for (int i = 0; i < 5; i++) l = insere(l, i);
    libera(l);
    palindromo("arara");
    int v[] = {1, 2, 3};
    printf("%d %d %d\n", procura(v, 3, 2), calc('+', 1, 2), maximo(v, 3));
    printf("%f %d\n", grau_para_rad(90), eh_primo(7));
    int x = 5, y = 0;
    double d = (double) x / 2;
    int arred = floor(d + 0.5);
    float f = d;
    printf("%d %f %d\n", arred, f, y);
    char nome[20];
    scanf("%19s", nome);
    int len = strlen(nome);
    size_t tam = strlen(nome);
    for (size_t i = tam; i > 0; i--) putchar(nome[i - 1]);
    for (int i = len - 1; i >= 0; i--) putchar(nome[i]);
    printf("%d %d\n", conta(3), menu());
    int notas[5];
    for (int i = 0; i < 5; i++) notas[i] = rand() % 11;
    int aprovados = 0;
    for (int i = 0; i < 5; i++) if (notas[i] >= 6) aprovados++;
    printf("%.1f%%\n", 100.0 * aprovados / 5);
    int pct = 100.0 * aprovados / 5;
    char letra = 'A' + 2;
    char maiusc = nome[0] - 32;
    printf("%d %c %c\n", pct, letra, maiusc);
    assert(x > 0);
    srand(time(NULL));
    return 0;
}
