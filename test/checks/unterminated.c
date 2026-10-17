#include <stdio.h>
#include <string.h>

int main(void) {
    char nome[5] = "Maria";
    if (strcmp(nome, "Maria") == 0)
        puts("achou");
    else
        puts("não achou");
}
