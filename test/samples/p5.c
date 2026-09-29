#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <time.h>
#include <errno.h>
#include <float.h>
#include <inttypes.h>
static double integra(double (*g)(double), double a, double b, int n) { double h = (b-a)/n, s = 0; for (int i = 0; i < n; i++) s += g(a + (i + .5) * h); return s * h; }
static double quad(double x) { return x * x; }
struct mat { int l, c; double *d; };
static struct mat nova(int l, int c) { return (struct mat){l, c, calloc(l * c, sizeof(double))}; }
int main(void) {
  struct mat m = nova(3, 3); for (int i = 0; i < 9; i++) m.d[i] = i;
  double tr = 0; for (int i = 0; i < 3; i++) tr += m.d[i * 3 + i];
  int64_t x = INT64_C(1) << 40; uint8_t by = 250; by += 10;
  char line[100]; int lidos = 0;
  while (fgets(line, sizeof line, stdin)) lidos += (int)strlen(line);
  printf("%.5f %.1f %" PRId64 " %u %d %g %d\n", integra(quad, 0, 1, 1000), tr, x, by, lidos, DBL_EPSILON, errno);
  free(m.d);
  return 0;
}
