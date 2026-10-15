#include <stdio.h>
int a1(int x) { int r; if (x > 0) r = 1; else if (x <= 0) r = 2; return r; }
int a2(int n) { int r; for (int i = 0; i < n; i++) r = i; return n > 0 ? r : 0; }
int a3(int n) { int achou = 0, pos; for (int i = 0; i < n; i++) if (i == 3) { achou = 1; pos = i; } if (achou) return pos; return -1; }
int a4(const int *v, int n) { int menor; for (int i = 0; i < n; i++) if (i == 0 || v[i] < menor) menor = v[i]; return menor; }
int a5(int op) { int r; switch (op) { case 0: r = 1; break; case 1: r = 2; break; } return r; }
double a6(int n) { double m; if (n > 0) { m = 1.0 / n; } printf("x\n"); if (n > 0) return m; return 0; }
int main(void) { return a1(1) + a2(1) + a3(5) + a4((int[]){1}, 1) + a5(0) + (int) a6(1); }
