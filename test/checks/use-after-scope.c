#include <stdio.h>

int main(void) {
    int *maior;
    {
        int x = 10;
        maior = &x;
    }
    printf("%d\n", *maior);
}
