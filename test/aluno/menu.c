// laço com menu, switch e leitura com scanf
#include <stdio.h>

int main(void) {
    int opcao;
    double saldo = 0, valor;
    char linha[50];
    while (fgets(linha, sizeof linha, stdin) != NULL && sscanf(linha, "%d", &opcao) == 1) {
        switch (opcao) {
        case 1:
            valor = 10;
            saldo += valor;
            break;
        case 2:
            if (saldo >= 5) {
                saldo -= 5;
            } else {
                printf("saldo insuficiente\n");
            }
            break;
        default:
            printf("opcao invalida\n");
            break;
        }
    }
    printf("saldo: %.2f\n", saldo);
    int idade = 20;
    const char *faixa = idade < 12 ? "crianca" : idade < 18 ? "adolescente" : "adulto";
    printf("%s\n", faixa);
    return 0;
}
