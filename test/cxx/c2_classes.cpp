// classes, inheritance, virtual functions, operator overloading
#include <iostream>
#include <vector>
#include <memory>
#include <cmath>
struct Shape {
  virtual ~Shape() { std::cout << "~Shape\n"; }
  virtual double area() const = 0;
  virtual const char *name() const { return "shape"; }
};
struct Circle : Shape {
  double r;
  explicit Circle(double r) : r(r) {}
  double area() const override { return M_PI * r * r; }
  const char *name() const override { return "circle"; }
};
struct Rect : Shape {
  double w, h;
  Rect(double w, double h) : w(w), h(h) {}
  double area() const override { return w * h; }
};
struct Vec {
  int x, y;
  Vec operator+(const Vec &o) const { return {x + o.x, y + o.y}; }
  bool operator==(const Vec &o) const { return x == o.x && y == o.y; }
};
std::ostream &operator<<(std::ostream &os, const Vec &v) { return os << '(' << v.x << ',' << v.y << ')'; }
int main() {
  std::vector<std::unique_ptr<Shape>> v;
  v.push_back(std::make_unique<Circle>(1.5));
  v.push_back(std::make_unique<Rect>(2, 3));
  for (auto &s : v) std::cout << s->name() << ' ' << s->area() << '\n';
  Vec a{1, 2}, b{3, 4};
  std::cout << a + b << ' ' << (a == b) << '\n';
  auto sp = std::make_shared<Rect>(1, 1);
  auto sp2 = sp;
  std::cout << sp.use_count() << '\n';
}
