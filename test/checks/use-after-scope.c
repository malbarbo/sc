#include <stdio.h>

int main(void) {
    int *largest;
    {
        int x = 10;
        largest = &x;
    }
    printf("%d\n", *largest);
}
