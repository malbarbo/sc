#include <stdio.h>
#include <string.h>

int main(void) {
    char name[5] = "Maria";
    if (strcmp(name, "Maria") == 0)
        puts("found");
    else
        puts("not found");
}
