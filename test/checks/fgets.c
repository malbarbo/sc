#include <stdio.h>

int main(void) {
    char name[10];
    if (fgets(name, 100, stdin) != NULL) {
        printf("%s", name);
    }
}
