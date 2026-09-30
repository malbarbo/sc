// C++20: concepts, requires, abbreviated templates, consteval
#include <concepts>
#include <iostream>
#include <string>

template <typename T>
concept Numeric = std::integral<T> || std::floating_point<T>;

template <typename T>
concept Printable = requires(std::ostream &os, T v) { os << v; };

Numeric auto sum(Numeric auto a, Numeric auto b) { return a + b; }

template <Printable T>
void show(const T &v) { std::cout << "[" << v << "]\n"; }

template <typename T>
  requires std::totally_ordered<T>
T maxOf(T a, T b) { return a < b ? b : a; }

consteval int square(int n) { return n * n; }
constexpr int fact(int n) { return n <= 1 ? 1 : n * fact(n - 1); }

int main() {
  std::cout << sum(2, 3) << " " << sum(1.5, 2.25) << "\n";
  show(42);
  show(std::string("texto"));
  std::cout << maxOf(3, 7) << " " << maxOf(std::string("abc"), std::string("abd")) << "\n";
  constexpr int s = square(12);
  static_assert(fact(5) == 120);
  std::cout << s << " " << fact(6) << "\n";
}
