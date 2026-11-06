#include <stdio.h>

void read_int(int *n) {
    *n = 42;
}

int main(int argc, char *argv[]) {
    int *p;
    if (argc > 5) {
        p = NULL;
    }
    read_int(p);
    printf("%d\n", *p);
}
