#include <stdio.h>

int main(void) {
    int votos[3];
    int urna[] = {0, 2, 1, 2, 2};
    for (int i = 0; i < 5; i++)
        votos[urna[i]]++;
    for (int c = 0; c < 3; c++)
        if (votos[c] < 0 || votos[c] > 5)
            printf("candidato %d: contagem impossível\n", c);
        else
            printf("candidato %d: %d votos\n", c, votos[c]);
}
