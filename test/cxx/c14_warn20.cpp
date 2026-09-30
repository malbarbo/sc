// warnings with -Wall -Wextra in C++20 code
#include <iostream>
#include <string>
#include <vector>
struct S {
  int a;
  bool operator==(const S &o) { return a == o.a; }
};
int g(int v) {
  switch (v) {
  case 1: std::cout << "um";
  case 2: std::cout << "dois"; break;
  }
  return v;
}
int main() {
  std::vector<std::string> nomes{"a", "b"};
  for (auto n : nomes) std::cout << n;
  int arr[3] = {1, 2, 3};
  int i = 0;
  arr[i] = i++;
  S s{1};
  bool eq = s == s;
  double d = 3.7;
  int k = d;
  std::string t = "x" + 1;
  return g(k) + eq + (arr[0] == 1.0) + t.size();
}
