#include <stdio.h>

int average(int sum, int n) {
    return sum / n;
}

int main(void) {
    int n = 0;
    printf("before\n");
    printf("%d\n", average(10, n));
    printf("after\n");
}
