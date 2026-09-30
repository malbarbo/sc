// C++20 compile errors: unsatisfied concepts, consteval, designated order
#include <concepts>
#include <string>
#include <vector>
#include <algorithm>

template <std::integral T>
T twice(T x) { return 2 * x; }

consteval int sq(int n) { return n * n; }

struct P { int x, y; };

int main(int argc, char **) {
  twice(2.5);
  twice(std::string("a"));
  int n = sq(argc);
  P p{.y = 1, .x = 2};
  std::vector<std::string> v;
  std::ranges::sort(v, [](int a, int b) { return a < b; });
  return n + p.x;
}
