#include <stdio.h>

int aplica(int (*f)(int), int x) {
    return f(x);
}

int main(void) {
    int (*f)(int) = NULL;
    printf("%d\n", aplica(f, 3));
}
