// strings: palindromes, counting vowels, upper case and reading lines
#include <ctype.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

bool is_palindrome(const char *s) {
    int i = 0, j = strlen(s) - 1;
    while (i < j) {
        if (tolower(s[i]) != tolower(s[j])) {
            return false;
        }
        i++;
        j--;
    }
    return true;
}

int vowels(const char *s) {
    int n = 0;
    for (int i = 0; s[i] != '\0'; i++) {
        if (strchr("aeiouAEIOU", s[i]) != NULL) {
            n++;
        }
    }
    return n;
}

void to_upper(char *s) {
    for (; *s; s++) {
        *s = toupper(*s);
    }
}

int main(void) {
    const char *words[] = {"level", "Ana", "house", "Otto"};
    for (int i = 0; i < 4; i++) {
        printf("%s: %s, %d vowels\n", words[i], is_palindrome(words[i]) ? "yes" : "no", vowels(words[i]));
    }
    char line[100];
    while (fgets(line, sizeof line, stdin) != NULL) {
        line[strcspn(line, "\n")] = '\0';
        to_upper(line);
        printf("%s (%zu)\n", line, strlen(line));
    }
    char name[20];
    strcpy(name, "maria");
    strcat(name, " silva");
    printf("%s\n", name);
    return 0;
}
