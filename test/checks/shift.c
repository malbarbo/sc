#include <stdio.h>

int main(void) {
    int bits = 32;
    unsigned mask = 1 << bits;
    printf("%u\n", mask);
}
