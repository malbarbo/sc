// C++20: std::format, coroutines, char8_t, [[likely]], using enum, constinit
#include <coroutine>
#include <format>
#include <iostream>
#include <map>
#include <string>

struct Gen {
  struct promise_type {
    int cur;
    Gen get_return_object() { return Gen{std::coroutine_handle<promise_type>::from_promise(*this)}; }
    std::suspend_always initial_suspend() { return {}; }
    std::suspend_always final_suspend() noexcept { return {}; }
    std::suspend_always yield_value(int v) { cur = v; return {}; }
    void return_void() {}
    void unhandled_exception() {}
  };
  std::coroutine_handle<promise_type> h;
  ~Gen() { h.destroy(); }
  bool next() { h.resume(); return !h.done(); }
  int value() { return h.promise().cur; }
};

Gen fib(int n) {
  int a = 0, b = 1;
  for (int i = 0; i < n; ++i) {
    co_yield a;
    int t = a + b; a = b; b = t;
  }
}

enum class Color { Red, Green, Blue };
constinit int counter = 10;

const char *name(Color c) {
  switch (c) {
    using enum Color;
    case Red: return "red";
    case Green: return "green";
    case Blue: return "blue";
  }
  return "?";
}

int main() {
  std::cout << std::format("{:>8}|{:<6}|{:^7}|\n", "right", "left", "mid");
  std::cout << std::format("{:.3f} {:08.2f} {:x} {:#b} {:e}\n", 3.14159, 2.5, 255, 5, 12345.678);
  std::map<std::string, int> m{{"a", 1}, {"b", 2}};
  for (auto &[k, v] : m) std::cout << std::format("{}={} ", k, v);
  std::cout << "\n";
  auto g = fib(10);
  while (g.next()) std::cout << g.value() << " ";
  std::cout << "\n";
  const char8_t *u = u8"ok";
  std::cout << (int)u[0] << " " << name(Color::Green) << " " << counter << "\n";
  int x = 5;
  if (x > 0) [[likely]] std::cout << "positive\n";
}
