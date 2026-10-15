#include <stdio.h>

int fatorial(int n) {
    return n * fatorial(n - 1);
}

int main(void) {
    printf("%d\n", fatorial(5));
}
