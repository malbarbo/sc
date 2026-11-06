#include <stdio.h>

void fill(int v[], int n) {
    for (int i = 0; i <= n; i++) {
        v[i] = 0;
    }
}

int main(void) {
    int grades[4];
    fill(grades, 4);
    printf("%d\n", grades[0]);
}
