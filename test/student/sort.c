#include <stdio.h>
#include <string.h>

typedef struct {
    char name[20];
    int score;
} Player;

void swap(Player *a, Player *b) {
    Player t = *a;
    *a = *b;
    *b = t;
}

// selection sort: the smallest is often already in place, so a[i] is
// swapped with itself
void sort(Player a[], int n) {
    for (int i = 0; i < n - 1; i++) {
        int min = i;
        for (int j = i + 1; j < n; j++)
            if (a[j].score < a[min].score) min = j;
        swap(&a[i], &a[min]);
    }
}

int main(void) {
    Player p[] = {{"ana", 3}, {"bia", 7}, {"caio", 5}, {"duda", 9}};
    int n = sizeof p / sizeof p[0];
    sort(p, n);
    for (int i = 0; i < n; i++) printf("%s %d\n", p[i].name, p[i].score);
    p[0] = p[0];
    printf("%s\n", p[0].name);
    return 0;
}
