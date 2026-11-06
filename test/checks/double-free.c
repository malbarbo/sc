#include <stdlib.h>
#include <string.h>

int main(void) {
    char *name = malloc(10);
    strcpy(name, "ana");
    free(name);
    free(name);
}
