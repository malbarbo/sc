#include <stdio.h>

int apply(int (*f)(int), int x) {
    return f(x);
}

int main(void) {
    int (*f)(int) = NULL;
    printf("%d\n", apply(f, 3));
}
