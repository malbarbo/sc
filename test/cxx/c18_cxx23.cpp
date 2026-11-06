// C++23 and C++26: std::print, std::expected, deducing this, std::ranges::to,
// if consteval, multidimensional subscript, pack indexing, = delete("reason"),
// the _ placeholder, static operator()
#include <algorithm>
#include <expected>
#include <map>
#include <print>
#include <ranges>
#include <string>
#include <vector>
std::expected<int, std::string> parse(const std::string &s) {
  if (s.empty() || !std::ranges::all_of(s, [](char c) { return c >= '0' && c <= '9'; }))
    return std::unexpected("not a number: '" + s + "'");
  return std::stoi(s);
}
struct Counter {
  int n = 0;
  template <class Self> auto &&get(this Self &&self) { return std::forward<Self>(self).n; }
  auto inc(this Counter &self) -> Counter & { ++self.n; return self; }
};
struct Grid {
  int cells[3][3]{};
  int &operator[](int r, int c) { return cells[r][c]; }
};
template <class... Ts> auto first(Ts... xs) { return xs...[0]; }
void legacy(int) = delete("use the new version");
struct Twice { static int operator()(int x) { return 2 * x; } };
constexpr int compile_or_run(int x) {
  if consteval { return x * 10; } else { return x; }
}
int main() {
  for (auto s : {"12", "x1", ""}) {
    auto r = parse(s);
    if (r) std::println("ok {}", *r); else std::println("error: {}", r.error());
  }
  auto r = parse("7").and_then([](int x) -> std::expected<int, std::string> { return x * 6; });
  std::println("and_then: {}", r.value_or(-1));
  Counter c;
  c.inc().inc().inc();
  std::println("counter {}", c.get());
  auto evens = std::views::iota(1, 11) | std::views::filter([](int x) { return x % 2 == 0; }) | std::ranges::to<std::vector>();
  std::println("evens {}", evens);
  std::map<std::string, int> m{{"a", 1}, {"b", 2}};
  auto keys = m | std::views::keys | std::ranges::to<std::vector<std::string>>();
  std::println("keys {} zip:", keys);
  for (auto [i, k] : std::views::zip(std::views::iota(0), keys)) std::print(" {}={}", i, k);
  std::println("");
  Grid g;
  g[1, 2] = 5;
  std::println("grid {}", g[1, 2]);
  std::println("first {}", first(3, 4.5, 'x'));
  std::println("twice {}", Twice{}(21));
  auto [_, b] = std::pair{1, 2};
  auto [_, d] = std::pair{3, 4};
  std::println("b + d = {}", b + d);
  constexpr int k = compile_or_run(4);
  int v = 4;
  std::println("consteval {} {}", k, compile_or_run(v));
  std::println("{:>8.3f}|{:<6}|{:^7}|{:#x}", 3.14159, "ab", "mid", 255);
}
