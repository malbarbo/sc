#include <stdio.h>
#include <string.h>

int length(const char *s) {
    return strlen(s);
}

int main(void) {
    printf("%d\n", length((const char *) 0xFFFFFFF0));
}
