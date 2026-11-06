#include <stdlib.h>

int main(void) {
    int age = 20;
    int *p = &age;
    free(p);
}
