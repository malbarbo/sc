// import std: format, ranges, optional, variant, smart pointers, algorithms
import std;
struct Shape { virtual ~Shape() = default; virtual double area() const = 0; };
struct Circle : Shape { double r; Circle(double r) : r(r) {} double area() const override { return 3.14159 * r * r; } };
std::optional<int> parse(std::string_view s) {
  int v = 0;
  for (char c : s) { if (c < '0' || c > '9') return std::nullopt; v = v * 10 + (c - '0'); }
  return v;
}
int main() {
  std::vector<int> v(10);
  std::iota(v.begin(), v.end(), 1);
  auto even = v | std::views::filter([](int x) { return x % 2 == 0; }) | std::views::transform([](int x) { return x * x; });
  for (int x : even) std::cout << x << ' ';
  std::cout << '\n' << std::format("{:>6.2f}|{:<4}|{}", 3.14159, 7, "fmt") << '\n';
  std::cout << parse("123").value_or(-1) << ' ' << parse("1x").value_or(-1) << '\n';
  std::variant<int, std::string> var = std::string("text");
  std::visit([](auto &&a) { std::cout << a << '\n'; }, var);
  std::vector<std::unique_ptr<Shape>> shapes;
  shapes.push_back(std::make_unique<Circle>(1.0));
  shapes.push_back(std::make_unique<Circle>(2.0));
  double total = std::accumulate(shapes.begin(), shapes.end(), 0.0, [](double s, auto &p) { return s + p->area(); });
  std::cout << std::fixed << std::setprecision(3) << total << ' ' << std::ranges::max(v) << ' ' << std::gcd(12, 18) << '\n';
  std::unordered_map<std::string, int> count;
  for (auto w : {"a", "b", "a"}) ++count[w];
  std::cout << count["a"] << count["b"] << ' ' << std::sqrt(2.0) << '\n';
}
