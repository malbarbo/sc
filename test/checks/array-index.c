#include <stdio.h>

int main(void) {
    int a[3] = {1, 2, 3};
    int soma = 0;
    for (int i = 0; i <= 3; i++) {
        soma += a[i];
    }
    printf("%d\n", soma);
}
