// an exception that no one catches: libc++abi prints its type and what()
#include <cstdio>
#include <stdexcept>
#include <vector>
int main() {
  std::vector<int> v{1, 2, 3};
  std::printf("%d\n", v.at(1));
  std::printf("%d\n", v.at(3));
}
