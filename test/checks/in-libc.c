#include <stdio.h>
#include <string.h>

int tamanho(const char *s) {
    return strlen(s);
}

int main(void) {
    printf("%d\n", tamanho((const char *) 0xFFFFFFF0));
}
