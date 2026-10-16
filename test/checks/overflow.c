#include <stdio.h>

int fatorial(int n) {
    int f = 1;
    for (int i = 2; i <= n; i++) {
        f = f * i;
    }
    return f;
}

int main(void) {
    for (int n = 10; n <= 15; n++) {
        printf("%d! = %d\n", n, fatorial(n));
    }
}
