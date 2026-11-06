// C++20: ranges, views, span, designated initializers, <=>
#include <algorithm>
#include <compare>
#include <iostream>
#include <ranges>
#include <span>
#include <string>
#include <vector>

struct Point {
  int x, y;
  auto operator<=>(const Point &) const = default;
};

struct Student {
  std::string name;
  double grade = 0;
  int absences = 0;
};

double average(std::span<const double> v) {
  double s = 0;
  for (double x : v) s += x;
  return v.empty() ? 0 : s / v.size();
}

int main() {
  std::vector<int> v{5, 3, 8, 1, 9, 2, 7};
  std::ranges::sort(v);
  for (int x : v | std::views::filter([](int x) { return x % 2; })
                 | std::views::transform([](int x) { return x * x; }))
    std::cout << x << " ";
  std::cout << "\n";
  for (int i : std::views::iota(1, 6) | std::views::reverse) std::cout << i;
  std::cout << "\n";
  auto it = std::ranges::find(v, 8);
  std::cout << (it - v.begin()) << " " << std::ranges::count_if(v, [](int x) { return x > 4; }) << "\n";

  Point a{1, 2}, b{1, 3};
  std::cout << (a < b) << (a == b) << ((a <=> b) < 0) << "\n";
  std::vector<Point> ps{{3, 1}, {1, 9}, {1, 2}};
  std::ranges::sort(ps);
  for (auto [x, y] : ps) std::cout << "(" << x << "," << y << ")";
  std::cout << "\n";

  Student st{.name = "Ana", .grade = 8.5};
  std::cout << st.name << " " << st.grade << " " << st.absences << "\n";
  double grades[] = {7.0, 8.0, 9.5};
  std::cout << average(grades) << "\n";
  std::string s = "programming";
  std::cout << s.starts_with("pro") << s.ends_with("ng") << "\n";
}
