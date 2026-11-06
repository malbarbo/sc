#include <stdio.h>

int counts[10];

int sum(int v[], int n) {
    int s = 0;
    for (int i = 1; i <= n; i++) {
        s += v[i];
    }
    return s;
}

int main(void) {
    printf("%d\n", sum(counts, 10));
}
