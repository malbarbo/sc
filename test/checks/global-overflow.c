#include <stdio.h>

int contagem[10];

int soma(int v[], int n) {
    int s = 0;
    for (int i = 1; i <= n; i++) {
        s += v[i];
    }
    return s;
}

int main(void) {
    printf("%d\n", soma(contagem, 10));
}
