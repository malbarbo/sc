#include <stdio.h>

int media(int soma, int n) {
    return soma / n;
}

int main(void) {
    int n = 0;
    printf("antes\n");
    printf("%d\n", media(10, n));
    printf("depois\n");
}
