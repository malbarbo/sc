// entry 2: catches what entry 1 throws, through the shared tag
#include <cstdio>
#include <stdexcept>
struct Problem : std::runtime_error { using std::runtime_error::runtime_error; };
int check(int x);
int main() {
  int xs[] = {3, -2};
  for (int x : xs) {
    try {
      std::printf("check %d\n", check(x));
    } catch (const Problem &p) {
      std::printf("Problem: %s\n", p.what());
    }
  }
  try { check(-5); } catch (const std::exception &e) { std::printf("as std::exception: %s\n", e.what()); }
}
