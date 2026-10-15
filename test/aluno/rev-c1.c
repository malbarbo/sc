#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <ctype.h>
#include <stdbool.h>

#define N 10

typedef struct {
    char nome[50];
    int idade;
    double nota;
} Aluno;

int fatorial(int n) {
    if (n <= 1) return 1;
    return n * fatorial(n - 1);
}

int fib(int n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

double media(const double v[], int n) {
    double s = 0;
    for (int i = 0; i < n; i++) s += v[i];
    return s / n;
}

int maior(int a, int b) { return a > b ? a : b; }

void troca(int *a, int *b) { int t = *a; *a = *b; *b = t; }

void bubble(int v[], int n) {
    for (int i = 0; i < n - 1; i++)
        for (int j = 0; j < n - 1 - i; j++)
            if (v[j] > v[j + 1]) troca(&v[j], &v[j + 1]);
}

int busca_binaria(const int v[], int n, int x) {
    int ini = 0, fim = n - 1;
    while (ini <= fim) {
        int meio = (ini + fim) / 2;
        if (v[meio] == x) return meio;
        else if (v[meio] < x) ini = meio + 1;
        else fim = meio - 1;
    }
    return -1;
}

int conta_vogais(const char *s) {
    int c = 0;
    for (size_t i = 0; i < strlen(s); i++) {
        char ch = tolower(s[i]);
        if (ch == 'a' || ch == 'e' || ch == 'i' || ch == 'o' || ch == 'u') c++;
    }
    return c;
}

void inverte(char *s) {
    int n = strlen(s);
    for (int i = 0; i < n / 2; i++) {
        char t = s[i];
        s[i] = s[n - 1 - i];
        s[n - 1 - i] = t;
    }
}

bool eh_primo(int n) {
    if (n < 2) return false;
    for (int i = 2; i * i <= n; i++)
        if (n % i == 0) return false;
    return true;
}

int dia_semana(int d) {
    switch (d) {
    case 1: printf("domingo\n"); break;
    case 2: printf("segunda\n"); break;
    default: printf("outro\n"); break;
    }
    return 0;
}

int main(int argc, char *argv[]) {
    int v[N] = {5, 3, 8, 1, 9, 2, 7, 4, 6, 0};
    bubble(v, N);
    for (int i = 0; i < N; i++) printf("%d ", v[i]);
    printf("\n%d\n", busca_binaria(v, N, 7));
    printf("%d %d\n", fatorial(5), fib(10));
    double notas[] = {7.5, 8.0, 9.5};
    printf("%.2f\n", media(notas, 3));
    char s[] = "Ola Mundo";
    printf("%d\n", conta_vogais(s));
    inverte(s);
    printf("%s\n", s);

    Aluno a = {"Maria", 20, 8.5};
    Aluno turma[3] = {0};
    turma[0] = a;
    strcpy(turma[1].nome, "Joao");
    printf("%s %d %.1f\n", turma[0].nome, turma[0].idade, turma[0].nota);

    int n;
    if (scanf("%d", &n) != 1) n = 5;
    int *p = malloc(n * sizeof(int));
    if (p == NULL) return 1;
    for (int i = 0; i < n; i++) p[i] = i * i;
    int soma = 0;
    for (int i = 0; i < n; i++) soma += p[i];
    free(p);
    printf("%d\n", soma);

    double x = sqrt(2.0);
    int arred = (int) round(x * 100);
    printf("%d\n", arred);
    int metade = n / 2;
    double r = n / 2.0;
    printf("%d %f\n", metade, r);
    float f = 3.14f;
    printf("%f\n", f);
    int k = 0;
    while (1) { if (++k > 10) break; }
    char linha[100];
    if (fgets(linha, sizeof linha, stdin)) printf("%s", linha);
    for (int i = 0; i < 5; i++) {
        if (eh_primo(i)) printf("%d primo\n", i);
    }
    printf("%zu\n", sizeof(Aluno));
    dia_semana(1);
    int m[3][3] = {{1, 2, 3}, {4, 5, 6}, {7, 8, 9}};
    int tr = 0;
    for (int i = 0; i < 3; i++) tr += m[i][i];
    printf("%d %d\n", tr, maior(3, 4));
    char c = getchar();
    if (c == 'q') return 0;
    unsigned int u = 10;
    for (unsigned int i = 0; i < u; i++) {}
    long long big = 1LL << 40;
    printf("%lld\n", big);
    int nota_int = 7.5 * 2;
    double porcent = soma * 100.0 / n;
    int pct = porcent;
    printf("%d %d\n", nota_int, pct);
    return 0;
}
