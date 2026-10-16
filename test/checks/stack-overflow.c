#include <stdio.h>

void preenche(int v[], int n) {
    for (int i = 0; i <= n; i++) {
        v[i] = 0;
    }
}

int main(void) {
    int notas[4];
    preenche(notas, 4);
    printf("%d\n", notas[0]);
}
