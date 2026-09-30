// warnings with -Wall -Wextra
#include <iostream>
#include <vector>
struct B { int x; B(int y) : x(x) {} };
int f(int a) { if (a) return 1; }
int main() {
  std::vector<int> v{1, 2};
  for (int i = 0; i < v.size(); ++i) std::cout << v[i];
  int unused;
  unsigned u = -1;
  if (u = 3) std::cout << u;
  B b(1);
  return f(b.x);
}
