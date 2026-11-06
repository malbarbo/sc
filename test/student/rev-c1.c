#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <ctype.h>
#include <stdbool.h>

#define N 10

typedef struct {
    char name[50];
    int age;
    double grade;
} Student;

int factorial(int n) {
    if (n <= 1) return 1;
    return n * factorial(n - 1);
}

int fib(int n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

double mean(const double v[], int n) {
    double s = 0;
    for (int i = 0; i < n; i++) s += v[i];
    return s / n;
}

int larger(int a, int b) { return a > b ? a : b; }

void swap(int *a, int *b) { int t = *a; *a = *b; *b = t; }

void bubble(int v[], int n) {
    for (int i = 0; i < n - 1; i++)
        for (int j = 0; j < n - 1 - i; j++)
            if (v[j] > v[j + 1]) swap(&v[j], &v[j + 1]);
}

int binary_search(const int v[], int n, int x) {
    int lo = 0, hi = n - 1;
    while (lo <= hi) {
        int mid = (lo + hi) / 2;
        if (v[mid] == x) return mid;
        else if (v[mid] < x) lo = mid + 1;
        else hi = mid - 1;
    }
    return -1;
}

int count_vowels(const char *s) {
    int c = 0;
    for (size_t i = 0; i < strlen(s); i++) {
        char ch = tolower(s[i]);
        if (ch == 'a' || ch == 'e' || ch == 'i' || ch == 'o' || ch == 'u') c++;
    }
    return c;
}

void reverse(char *s) {
    int n = strlen(s);
    for (int i = 0; i < n / 2; i++) {
        char t = s[i];
        s[i] = s[n - 1 - i];
        s[n - 1 - i] = t;
    }
}

bool is_prime(int n) {
    if (n < 2) return false;
    for (int i = 2; i * i <= n; i++)
        if (n % i == 0) return false;
    return true;
}

int weekday(int d) {
    switch (d) {
    case 1: printf("Sunday\n"); break;
    case 2: printf("Monday\n"); break;
    default: printf("other\n"); break;
    }
    return 0;
}

int main(int argc, char *argv[]) {
    int v[N] = {5, 3, 8, 1, 9, 2, 7, 4, 6, 0};
    bubble(v, N);
    for (int i = 0; i < N; i++) printf("%d ", v[i]);
    printf("\n%d\n", binary_search(v, N, 7));
    printf("%d %d\n", factorial(5), fib(10));
    double grades[] = {7.5, 8.0, 9.5};
    printf("%.2f\n", mean(grades, 3));
    char s[] = "Hello World";
    printf("%d\n", count_vowels(s));
    reverse(s);
    printf("%s\n", s);

    Student a = {"Maria", 20, 8.5};
    Student group[3] = {0};
    group[0] = a;
    strcpy(group[1].name, "Joao");
    printf("%s %d %.1f\n", group[0].name, group[0].age, group[0].grade);

    int n;
    if (scanf("%d", &n) != 1) n = 5;
    int *p = malloc(n * sizeof(int));
    if (p == NULL) return 1;
    for (int i = 0; i < n; i++) p[i] = i * i;
    int sum = 0;
    for (int i = 0; i < n; i++) sum += p[i];
    free(p);
    printf("%d\n", sum);

    double x = sqrt(2.0);
    int rounded = (int) round(x * 100);
    printf("%d\n", rounded);
    int half = n / 2;
    double r = n / 2.0;
    printf("%d %f\n", half, r);
    float f = 3.14f;
    printf("%f\n", f);
    int k = 0;
    while (1) { if (++k > 10) break; }
    char line[100];
    if (fgets(line, sizeof line, stdin)) printf("%s", line);
    for (int i = 0; i < 5; i++) {
        if (is_prime(i)) printf("%d prime\n", i);
    }
    printf("%zu\n", sizeof(Student));
    weekday(1);
    int m[3][3] = {{1, 2, 3}, {4, 5, 6}, {7, 8, 9}};
    int tr = 0;
    for (int i = 0; i < 3; i++) tr += m[i][i];
    printf("%d %d\n", tr, larger(3, 4));
    char c = getchar();
    if (c == 'q') return 0;
    unsigned int u = 10;
    for (unsigned int i = 0; i < u; i++) {}
    long long big = 1LL << 40;
    printf("%lld\n", big);
    int grade_int = 7.5 * 2;
    double percent = sum * 100.0 / n;
    int pct = percent;
    printf("%d %d\n", grade_int, pct);
    return 0;
}
