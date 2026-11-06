#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define N 8
static int board[N][N];
static int ok(int r, int c) { for (int i = 0; i < r; i++) if (board[i][c] || (c-(r-i) >= 0 && board[i][c-(r-i)]) || (c+(r-i) < N && board[i][c+(r-i)])) return 0; return 1; }
static int queens(int r) { if (r == N) return 1; int t = 0; for (int c = 0; c < N; c++) if (ok(r, c)) { board[r][c] = 1; t += queens(r + 1); board[r][c] = 0; } return t; }
typedef struct { char name[32]; double grade; } Student;
int main(void) {
  Student a[3] = {{"ana", 9.5}, {"bia", 7.25}, {"caio", 8}};
  double m = 0; for (int i = 0; i < 3; i++) m += a[i].grade;
  char *s = strdup("abc,def,ghi"); int k = 0; for (char *t = strtok(s, ","); t; t = strtok(NULL, ",")) k++;
  unsigned long h = 5381; for (const char *p = "hash"; *p; p++) h = h * 33 + (unsigned char)*p;
  float f = 0.1f; double d = f * 3; long double ld = d / 7;
  printf("%d %.3f %d %lu %.6f %.6Lf\n", queens(0), m / 3, k, h, d, ld);
  free(s);
  return 0;
}
