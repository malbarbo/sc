// recursão: fatorial, fibonacci com memória, potência, hanoi e mdc
#include <stdio.h>

long long fatorial(int n) {
    return n <= 1 ? 1 : n * fatorial(n - 1);
}

long long memo[91];

long long fib(int n) {
    if (n < 2) return n;
    if (memo[n] == 0) memo[n] = fib(n - 1) + fib(n - 2);
    return memo[n];
}

double potencia(double b, int e) {
    if (e == 0) return 1;
    double m = potencia(b, e / 2);
    return e % 2 == 0 ? m * m : m * m * b;
}

void hanoi(int n, char de, char para, char via) {
    if (n == 0) return;
    hanoi(n - 1, de, via, para);
    printf("%d: %c -> %c\n", n, de, para);
    hanoi(n - 1, via, para, de);
}

int mdc(int a, int b) {
    return b == 0 ? a : mdc(b, a % b);
}

int main(void) {
    printf("%lld %lld %.1f %d\n", fatorial(15), fib(90), potencia(2, 10), mdc(84, 36));
    hanoi(3, 'A', 'C', 'B');
    return 0;
}
