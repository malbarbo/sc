/* C23: nullptr, constexpr, typeof, bool, true and false as keywords,
   [[attributes]], static_assert, binary literals and digit separators,
   auto, enums with a fixed type, #elifdef, empty initializers, unnamed
   parameters, u8 characters */
#include <stdio.h>
#include <stddef.h>
#include <stdint.h>
#include <limits.h>
constexpr int SIZE = 4;
constexpr double PI = 3.14159;
enum Color : uint8_t { RED = 1, GREEN = 2, BLUE = 0b1000'0000 };
static_assert(sizeof(enum Color) == 1);
static_assert(SIZE == 4, "size");
[[nodiscard]] static int square(int x) { return x * x; }
[[maybe_unused]] static int unused_helper(int) { return 0; }
static void swap_any(void *a, void *b, size_t n) {
  unsigned char *x = a, *y = b;
  for (size_t i = 0; i < n; i++) { unsigned char t = x[i]; x[i] = y[i]; y[i] = t; }
}
#define SWAP(a, b) swap_any(&(a), &(b), sizeof(typeof(a)))
#ifdef NOT_DEFINED
#error no
#elifdef SIZE_MAX
#define HAS_SIZE_MAX 1
#endif
int main(void) {
  int *p = nullptr;
  bool ok = p == nullptr;
  printf("ok %d true %d sizeof(nullptr) %zu\n", ok, true, sizeof nullptr);
  int arr[SIZE] = {};
  for (int i = 0; i < SIZE; i++) arr[i] = square(i);
  auto total = 0;
  for (int i = 0; i < SIZE; i++) total += arr[i];
  printf("total %d pi %.2f\n", total, PI);
  typeof(total) a = 1'000'000, b = 2;
  SWAP(a, b);
  printf("a %d b %d\n", a, b);
  typeof_unqual(const int) m = 5;
  m++;
  printf("m %d blue %d bits %d\n", m, BLUE, BOOL_WIDTH);
  printf("u8 %d has_size_max %d\n", u8'A', HAS_SIZE_MAX);
  printf("version %ld\n", __STDC_VERSION__);
  return 0;
}
