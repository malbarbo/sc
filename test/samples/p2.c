#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define N 8
static int tab[N][N];
static int ok(int r, int c) { for (int i = 0; i < r; i++) if (tab[i][c] || (c-(r-i) >= 0 && tab[i][c-(r-i)]) || (c+(r-i) < N && tab[i][c+(r-i)])) return 0; return 1; }
static int rainhas(int r) { if (r == N) return 1; int t = 0; for (int c = 0; c < N; c++) if (ok(r, c)) { tab[r][c] = 1; t += rainhas(r + 1); tab[r][c] = 0; } return t; }
typedef struct { char nome[32]; double nota; } Aluno;
int main(void) {
  Aluno a[3] = {{"ana", 9.5}, {"bia", 7.25}, {"caio", 8}};
  double m = 0; for (int i = 0; i < 3; i++) m += a[i].nota;
  char *s = strdup("abc,def,ghi"); int k = 0; for (char *t = strtok(s, ","); t; t = strtok(NULL, ",")) k++;
  unsigned long h = 5381; for (const char *p = "hash"; *p; p++) h = h * 33 + (unsigned char)*p;
  float f = 0.1f; double d = f * 3; long double ld = d / 7;
  printf("%d %.3f %d %lu %.6f %.6Lf\n", rainhas(0), m / 3, k, h, d, ld);
  free(s);
  return 0;
}
