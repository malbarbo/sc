// matrizes: leitura de valores fixos, transposta, produto e diagonal
#include <stdio.h>

#define N 3

void imprime(int m[N][N]) {
    for (int i = 0; i < N; i++) {
        for (int j = 0; j < N; j++) {
            printf("%4d", m[i][j]);
        }
        printf("\n");
    }
}

void transposta(int m[N][N], int t[N][N]) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++)
            t[j][i] = m[i][j];
}

void produto(int a[N][N], int b[N][N], int c[N][N]) {
    for (int i = 0; i < N; i++) {
        for (int j = 0; j < N; j++) {
            c[i][j] = 0;
            for (int k = 0; k < N; k++) {
                c[i][j] += a[i][k] * b[k][j];
            }
        }
    }
}

int soma_diagonal(int m[N][N]) {
    int soma = 0;
    for (int i = 0; i < N; i++) {
        soma += m[i][i];
    }
    return soma;
}

int main(void) {
    int a[N][N] = {{1, 2, 3}, {4, 5, 6}, {7, 8, 9}};
    int t[N][N], c[N][N];
    transposta(a, t);
    produto(a, t, c);
    imprime(c);
    printf("diagonal: %d\n", soma_diagonal(c));
    return 0;
}
