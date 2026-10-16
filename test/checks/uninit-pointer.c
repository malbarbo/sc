#include <stdio.h>

void le(int *n) {
    *n = 42;
}

int main(int argc, char *argv[]) {
    int *p;
    if (argc > 5) {
        p = NULL;
    }
    le(p);
    printf("%d\n", *p);
}
