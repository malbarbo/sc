#include <stdio.h>

int main(void) {
    char nome[10];
    if (fgets(nome, 100, stdin) != NULL) {
        printf("%s", nome);
    }
}
