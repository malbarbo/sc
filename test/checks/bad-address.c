#include <stdio.h>

int main(void) {
    int *p = (int *) 0xFFFFFFF0;
    printf("%d\n", *p);
}
