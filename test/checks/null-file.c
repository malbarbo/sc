#include <stdio.h>

int main(void) {
    char line[100];
    FILE *f = fopen("notes.txt", "r");
    while (fgets(line, sizeof line, f) != NULL) {
        printf("%s", line);
    }
    fclose(f);
}
