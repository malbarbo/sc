#include <stdlib.h>

int main(void) {
    int idade = 20;
    int *p = &idade;
    free(p);
}
