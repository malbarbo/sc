// compile errors a student makes: the diagnostics must match
#include <iostream>
#include <vector>
struct A { virtual void f() = 0; };
int main() {
  std::vector<int> v;
  v.push_back("x");
  A a;
  int n = undefined_name;
  std::cout << v << '\n';
  for (auto x : 5) {}
  return "no";
}
