// matrices: fixed values, transpose, product and diagonal
#include <stdio.h>

#define N 3

void print(int m[N][N]) {
    for (int i = 0; i < N; i++) {
        for (int j = 0; j < N; j++) {
            printf("%4d", m[i][j]);
        }
        printf("\n");
    }
}

void transpose(int m[N][N], int t[N][N]) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++)
            t[j][i] = m[i][j];
}

void product(int a[N][N], int b[N][N], int c[N][N]) {
    for (int i = 0; i < N; i++) {
        for (int j = 0; j < N; j++) {
            c[i][j] = 0;
            for (int k = 0; k < N; k++) {
                c[i][j] += a[i][k] * b[k][j];
            }
        }
    }
}

int diagonal_sum(int m[N][N]) {
    int sum = 0;
    for (int i = 0; i < N; i++) {
        sum += m[i][i];
    }
    return sum;
}

int main(void) {
    int a[N][N] = {{1, 2, 3}, {4, 5, 6}, {7, 8, 9}};
    int t[N][N], c[N][N];
    transpose(a, t);
    product(a, t, c);
    print(c);
    printf("diagonal: %d\n", diagonal_sum(c));
    return 0;
}
