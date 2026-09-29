#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <ctype.h>
#include <assert.h>
#include <limits.h>
typedef struct no { int v; struct no *prox; } No;
typedef union { float f; uint32_t u; } FU;
enum cor { VERM, VERDE = 5, AZUL };
struct bits { unsigned a:3, b:5; };
static int soma(int n, ...) { va_list ap; va_start(ap, n); int s = 0; for (int i = 0; i < n; i++) s += va_arg(ap, int); va_end(ap); return s; }
static int cmp(const void *a, const void *b) { return *(const int*)a - *(const int*)b; }
static No *push(No *l, int v) { No *n = malloc(sizeof *n); n->v = v; n->prox = l; return n; }
#define TIPO(x) _Generic((x), int: "int", double: "double", default: "outro")
_Static_assert(sizeof(int) == 4, "int");
int main(void) {
  No *l = NULL; for (int i = 0; i < 5; i++) l = push(l, i * i);
  int s = 0; for (No *p = l; p; p = p->prox) s += p->v;
  int v[] = {5, 3, 9, 1, 7}; qsort(v, 5, sizeof v[0], cmp);
  FU fu = {.f = 1.0f}; struct bits bb = {5, 17};
  int n = 4; int vla[n]; for (int i = 0; i < n; i++) vla[i] = i;
  char buf[64]; snprintf(buf, sizeof buf, "%d-%s", soma(3, 1, 2, 3), TIPO(2.0));
  int (*fp)(int, ...) = soma;
  switch (AZUL) { case VERM: puts("r"); break; case AZUL: puts("azul"); break; default: break; }
  long long big = LLONG_MAX / 3; double r = sqrt(2.0) * pow(2, 10) + sin(0.5);
  for (char *c = buf; *c; c++) *c = toupper((unsigned char)*c);
  assert(bb.a == 5);
  printf("%d %d %d %x %u %s %d %lld %.4f %s\n", s, v[0], v[4], fu.u, bb.b, buf, fp(2, 10, 20), big, r, (bool)1 ? "sim" : "nao");
  goto fim;
fim:
  return vla[3] + (int)(struct { int a; }){7}.a - 10;
}
