#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <assert.h>
#include <time.h>

int global_n = 0;
int total;

typedef struct Node { int value; struct Node *next; } Node;

Node *insert(Node *list, int value) {
    Node *node = malloc(sizeof(Node));
    node->value = value;
    node->next = list;
    return node;
}

void free_list(Node *list) {
    while (list != NULL) { Node *t = list; list = list->next; free(t); }
}

int is_prime(int n) {
    int limit = sqrt(n);
    for (int i = 2; i <= limit; i++) if (n % i == 0) return 0;
    return n > 1;
}

void palindrome(const char *s) {
    int i, j;
    for (i = 0, j = strlen(s) - 1; i < j; i++, j--)
        if (s[i] != s[j]) { printf("no\n"); return; }
    printf("yes\n");
}

int menu(void) {
    int op;
    while (1) {
        printf("1-a 2-b 0-quit\n");
        if (scanf("%d", &op) != 1) return 0;
        switch (op) {
        case 0: return 0;
        case 1: printf("a\n"); break;
        case 2: printf("b\n"); break;
        default: printf("invalid\n");
        }
    }
    return 0;
}

int find(int *v, int n, int x) {
    for (int i = 0; i < n; i++) {
        if (v[i] == x) return i;
        else continue;
    }
    return -1;
}

int calc(char op, int a, int b) {
    int r;
    switch (op) {
    case '+': r = a + b; break;
    case '-': r = a - b; break;
    case '*': r = a * b; break;
    default: printf("error\n"); exit(1);
    }
    return r;
}

int maximum(int v[], int n) {
    int max;
    for (int i = 0; i < n; i++)
        if (i == 0 || v[i] > max) max = v[i];
    return max;
}

double deg_to_rad(double g) { return g * M_PI / 180.0; }

int count(int n) {
    int total = 0;
    for (int i = 0; i < n; i++) total += i;
    return total;
}

void fill(int n) {
    for (int i = 0; i < n; i++) {
        for (int i = 0; i < 2; i++) printf("x");
    }
}

int main(void) {
    Node *l = NULL;
    for (int i = 0; i < 5; i++) l = insert(l, i);
    free_list(l);
    palindrome("arara");
    int v[] = {1, 2, 3};
    printf("%d %d %d\n", find(v, 3, 2), calc('+', 1, 2), maximum(v, 3));
    printf("%f %d\n", deg_to_rad(90), is_prime(7));
    int x = 5, y = 0;
    double d = (double) x / 2;
    int rounded = floor(d + 0.5);
    float f = d;
    printf("%d %f %d\n", rounded, f, y);
    char name[20];
    scanf("%19s", name);
    int len = strlen(name);
    size_t size = strlen(name);
    for (size_t i = size; i > 0; i--) putchar(name[i - 1]);
    for (int i = len - 1; i >= 0; i--) putchar(name[i]);
    printf("%d %d\n", count(3), menu());
    int grades[5];
    for (int i = 0; i < 5; i++) grades[i] = rand() % 11;
    int passed = 0;
    for (int i = 0; i < 5; i++) if (grades[i] >= 6) passed++;
    printf("%.1f%%\n", 100.0 * passed / 5);
    int pct = 100.0 * passed / 5;
    char letter = 'A' + 2;
    char upper = name[0] - 32;
    printf("%d %c %c\n", pct, letter, upper);
    assert(x > 0);
    srand(time(NULL));
    return 0;
}
