// strings: palíndromo, contagem de vogais, maiúsculas e leitura de linhas
#include <ctype.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

bool palindromo(const char *s) {
    int i = 0, j = strlen(s) - 1;
    while (i < j) {
        if (tolower(s[i]) != tolower(s[j])) {
            return false;
        }
        i++;
        j--;
    }
    return true;
}

int vogais(const char *s) {
    int n = 0;
    for (int i = 0; s[i] != '\0'; i++) {
        if (strchr("aeiouAEIOU", s[i]) != NULL) {
            n++;
        }
    }
    return n;
}

void maiusculas(char *s) {
    for (; *s; s++) {
        *s = toupper(*s);
    }
}

int main(void) {
    const char *palavras[] = {"arara", "Ana", "casa", "Osso"};
    for (int i = 0; i < 4; i++) {
        printf("%s: %s, %d vogais\n", palavras[i], palindromo(palavras[i]) ? "sim" : "nao", vogais(palavras[i]));
    }
    char linha[100];
    while (fgets(linha, sizeof linha, stdin) != NULL) {
        linha[strcspn(linha, "\n")] = '\0';
        maiusculas(linha);
        printf("%s (%zu)\n", linha, strlen(linha));
    }
    char nome[20];
    strcpy(nome, "maria");
    strcat(nome, " silva");
    printf("%s\n", nome);
    return 0;
}
