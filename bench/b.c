#include <stdio.h>
#include <stdlib.h>
#define N 9
static int tab[N][N];
static int ok(int r, int c) { for (int i = 0; i < r; i++) if (tab[i][c] || (c-(r-i) >= 0 && tab[i][c-(r-i)]) || (c+(r-i) < N && tab[i][c+(r-i)])) return 0; return 1; }
static int rainhas(int r) { if (r == N) return 1; int t = 0; for (int c = 0; c < N; c++) if (ok(r, c)) { tab[r][c] = 1; t += rainhas(r + 1); tab[r][c] = 0; } return t; }
static int fib(int n) { return n < 2 ? n : fib(n-1) + fib(n-2); }
static void bolha(int *v, int n) { for (int i = 0; i < n; i++) for (int j = 0; j + 1 < n - i; j++) if (v[j] > v[j+1]) { int t = v[j]; v[j] = v[j+1]; v[j+1] = t; } }
int main(void) {
  int s = 0; for (int k = 0; k < 20; k++) s += rainhas(0);
  int n = 6000; int *v = malloc(n * sizeof *v); for (int i = 0; i < n; i++) v[i] = (i * 7919) % n; bolha(v, n);
  double acc = 0; for (int i = 1; i < 20000000; i++) acc += 1.0 / i;
  printf("%d %d %d %.6f\n", s, fib(30), v[n/2], acc);
  return 0;
}
