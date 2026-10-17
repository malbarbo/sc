#include <stdio.h>

int main(void) {
    int votos[3];
    int urna[] = {0, 2, 1, 2, 2};
    for (int i = 0; i < 5; i++)
        votos[urna[i]]++;
    printf("votos: %d %d %d\n", votos[0], votos[1], votos[2]);
}
