// exceptions: throw and catch by value and by reference, a hierarchy, the
// library's exceptions, RAII while unwinding, rethrow, function-try-block,
// exception_ptr, nested exceptions
#include <cstdio>
#include <exception>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
#include <map>
struct Trace {
  const char *n;
  explicit Trace(const char *n) : n(n) { std::printf("  + %s\n", n); }
  ~Trace() { std::printf("  - %s\n", n); }
};
struct AppError : std::runtime_error {
  int code;
  AppError(const std::string &m, int c) : std::runtime_error(m), code(c) {}
};
struct ParseError : AppError { using AppError::AppError; };
int depth(int n) {
  Trace t(n % 2 ? "odd" : "even");
  if (n == 0) throw ParseError("bottom", 42);
  return depth(n - 1) + 1;
}
struct Widget {
  std::vector<int> v;
  Widget(int n) try : v(n < 0 ? throw std::invalid_argument("n < 0") : n) {
  } catch (const std::exception &e) {
    std::printf("Widget: %s\n", e.what());
  }
};
double divide(double a, double b) {
  if (b == 0) throw std::domain_error("division by zero");
  return a / b;
}
int main() {
  try { depth(3); } catch (const AppError &e) { std::printf("AppError '%s' code %d\n", e.what(), e.code); }
  try { throw 7; } catch (long) { std::puts("long?"); } catch (int i) { std::printf("int %d\n", i); }
  try { throw std::string("text"); } catch (std::string s) { std::printf("string %s\n", s.c_str()); }
  try { std::vector<int> v(3); v.at(10) = 1; } catch (const std::out_of_range &) { std::puts("out_of_range"); }
  try { std::stoi("abc"); } catch (const std::invalid_argument &e) { std::printf("invalid_argument: %s\n", e.what()); }
  try { std::map<int, int> m; m.at(1); } catch (const std::exception &e) { std::puts("map::at"); }
  try { auto p = std::make_unique<int[]>(1); (void)p; throw std::bad_alloc(); } catch (const std::bad_alloc &e) { std::printf("bad_alloc: %s\n", e.what()); }
  try { std::printf("%g\n", divide(1, 4)); divide(1, 0); } catch (const std::logic_error &e) { std::printf("logic_error: %s\n", e.what()); }
  try {
    try { throw ParseError("inner", 1); } catch (AppError &e) { e.code = 2; std::puts("rethrow"); throw; }
  } catch (const ParseError &e) { std::printf("outer code %d\n", e.code); }
  try { Widget w(-1); } catch (const std::invalid_argument &e) { std::printf("rethrown by the function-try-block: %s\n", e.what()); }
  std::exception_ptr ep;
  try { throw std::runtime_error("stored"); } catch (...) { ep = std::current_exception(); }
  try { std::rethrow_exception(ep); } catch (const std::exception &e) { std::printf("exception_ptr: %s\n", e.what()); }
  try {
    try { throw std::runtime_error("cause"); } catch (...) { std::throw_with_nested(std::logic_error("effect")); }
  } catch (const std::logic_error &e) {
    std::printf("nested: %s\n", e.what());
    try { std::rethrow_if_nested(e); } catch (const std::runtime_error &c) { std::printf("  cause: %s\n", c.what()); }
  }
  std::printf("uncaught: %d\n", std::uncaught_exceptions());
}
