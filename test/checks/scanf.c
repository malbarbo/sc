#include <stdio.h>

int main(void) {
    char nome[8];
    int idade;
    printf("Nome e idade: ");
    if (scanf("%s %d", nome, &idade) == 2) {
        printf("%s tem %d anos\n", nome, idade);
    }
}
