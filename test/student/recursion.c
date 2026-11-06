// recursion: factorial, fibonacci with memoization, power, hanoi and gcd
#include <stdio.h>

long long factorial(int n) {
    return n <= 1 ? 1 : n * factorial(n - 1);
}

long long memo[91];

long long fib(int n) {
    if (n < 2) return n;
    if (memo[n] == 0) memo[n] = fib(n - 1) + fib(n - 2);
    return memo[n];
}

double power(double b, int e) {
    if (e == 0) return 1;
    double m = power(b, e / 2);
    return e % 2 == 0 ? m * m : m * m * b;
}

void hanoi(int n, char from, char to, char via) {
    if (n == 0) return;
    hanoi(n - 1, from, via, to);
    printf("%d: %c -> %c\n", n, from, to);
    hanoi(n - 1, via, to, from);
}

int gcd(int a, int b) {
    return b == 0 ? a : gcd(b, a % b);
}

int main(void) {
    printf("%lld %lld %.1f %d\n", factorial(15), fib(90), power(2, 10), gcd(84, 36));
    hanoi(3, 'A', 'C', 'B');
    return 0;
}
