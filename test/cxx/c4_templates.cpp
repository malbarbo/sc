// templates, lambdas, std::function, optional/variant, constexpr
#include <array>
#include <functional>
#include <iostream>
#include <optional>
#include <string>
#include <string_view>
#include <tuple>
#include <variant>
template <typename T> T maxOf(T a, T b) { return a < b ? b : a; }
template <typename T, int N> struct Stack {
  std::array<T, N> data{};
  int top = 0;
  void push(T x) { data[top++] = x; }
  T pop() { return data[--top]; }
  bool empty() const { return top == 0; }
};
template <typename... Ts> auto sum(Ts... xs) { return (xs + ... + 0); }
constexpr int fact(int n) { return n <= 1 ? 1 : n * fact(n - 1); }
std::optional<int> parse(std::string_view s) {
  int r = 0;
  for (char c : s) { if (c < '0' || c > '9') return std::nullopt; r = r * 10 + (c - '0'); }
  return r;
}
int main() {
  std::cout << maxOf(3, 7) << ' ' << maxOf(std::string("abc"), std::string("abd")) << '\n';
  Stack<double, 8> s;
  for (int i = 0; i < 4; ++i) s.push(i * 1.5);
  while (!s.empty()) std::cout << s.pop() << ' ';
  std::cout << '\n' << sum(1, 2, 3, 4) << ' ' << fact(10) << '\n';
  static_assert(fact(5) == 120);
  int base = 10;
  std::function<int(int)> f = [base](int x) { return x + base; };
  auto g = [&](int x) mutable { base++; return f(x) * 2; };
  std::cout << g(1) << ' ' << base << '\n';
  std::cout << parse("123").value_or(-1) << ' ' << parse("1x").has_value() << '\n';
  std::variant<int, std::string> v = 42;
  std::cout << std::get<int>(v) << ' ';
  v = "text";
  std::visit([](auto &&x) { std::cout << x << '\n'; }, v);
  auto [a, b, c] = std::make_tuple(1, 'z', 2.5);
  std::cout << a << b << c << '\n';
}
