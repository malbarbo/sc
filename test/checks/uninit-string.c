#include <stdio.h>

int main(int argc, char *argv[]) {
    char *name;
    if (argc > 5) {
        name = argv[1];
    }
    printf("%c\n", name[0]);
}
