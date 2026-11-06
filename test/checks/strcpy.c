#include <stdio.h>
#include <stdlib.h>
#include <string.h>

char *copy(const char *s) {
    char *c = malloc(strlen(s));
    strcpy(c, s);
    return c;
}

int main(void) {
    char *name = copy("maria");
    printf("%s\n", name);
    free(name);
}
