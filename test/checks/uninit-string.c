#include <stdio.h>

int main(int argc, char *argv[]) {
    char *nome;
    if (argc > 5) {
        nome = argv[1];
    }
    printf("%c\n", nome[0]);
}
