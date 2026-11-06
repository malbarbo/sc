// import std with exceptions, std::print and std::expected
import std;
struct Empty : std::runtime_error { Empty() : std::runtime_error("empty stack") {} };
template <class T> class Stack {
  std::vector<T> items;
public:
  void push(T x) { items.push_back(std::move(x)); }
  T pop() {
    if (items.empty()) throw Empty{};
    T x = std::move(items.back());
    items.pop_back();
    return x;
  }
};
std::expected<double, std::string> safe_sqrt(double x) {
  if (x < 0) return std::unexpected(std::format("negative: {}", x));
  return std::sqrt(x);
}
int main() {
  Stack<std::string> s;
  s.push("a");
  s.push("b");
  try {
    for (;;) std::println("pop {}", s.pop());
  } catch (const Empty &e) {
    std::println("end: {}", e.what());
  }
  for (double x : {16.0, -1.0}) {
    auto r = safe_sqrt(x);
    std::println("{}", r ? std::format("{}", *r) : r.error());
  }
  try { std::vector<int>{}.at(0); } catch (const std::out_of_range &) { std::println("out_of_range"); }
}
