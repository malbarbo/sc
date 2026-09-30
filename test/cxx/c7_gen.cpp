// class templates with inheritance, iterators, operator[], friends, move
#include <iostream>
#include <string>
#include <utility>
#include <vector>
template <typename T> class Matrix {
  int r, c;
  std::vector<T> d;
public:
  Matrix(int r, int c) : r(r), c(c), d(r * c) {}
  T &operator()(int i, int j) { return d[i * c + j]; }
  const T &operator()(int i, int j) const { return d[i * c + j]; }
  Matrix operator*(const Matrix &o) const {
    Matrix m(r, o.c);
    for (int i = 0; i < r; ++i)
      for (int j = 0; j < o.c; ++j)
        for (int k = 0; k < c; ++k) m(i, j) += (*this)(i, k) * o(k, j);
    return m;
  }
  friend std::ostream &operator<<(std::ostream &os, const Matrix &m) {
    for (int i = 0; i < m.r; ++i) { for (int j = 0; j < m.c; ++j) os << m(i, j) << ' '; os << '\n'; }
    return os;
  }
};
template <typename T> struct LinkedList {
  struct N { T v; N *n; };
  N *h = nullptr;
  ~LinkedList() { while (h) { N *t = h->n; delete h; h = t; } }
  void add(T v) { h = new N{std::move(v), h}; }
  struct It { N *p; T &operator*() { return p->v; } It &operator++() { p = p->n; return *this; } bool operator!=(It o) { return p != o.p; } };
  It begin() { return {h}; }
  It end() { return {nullptr}; }
};
struct Animal { std::string n; virtual std::string speak() const { return n + " ..."; } virtual ~Animal() = default; };
struct Dog : Animal { std::string speak() const override { return n + " woof"; } };
int main() {
  Matrix<int> a(2, 2), b(2, 2);
  a(0, 0) = 1; a(0, 1) = 2; a(1, 0) = 3; a(1, 1) = 4;
  b(0, 0) = 0; b(0, 1) = 1; b(1, 0) = 1; b(1, 1) = 0;
  std::cout << a * b;
  LinkedList<std::string> l;
  l.add("x"); l.add("y"); l.add("z");
  for (auto &s : l) std::cout << s;
  std::cout << '\n';
  Dog d; d.n = "rex";
  const Animal &r = d;
  std::cout << r.speak() << ' ' << (dynamic_cast<const Dog *>(&r) != nullptr) << ' ' << typeid(r).name() << '\n';
  std::vector<std::string> v;
  std::string s = "moved";
  v.push_back(std::move(s));
  v.emplace_back(3, 'k');
  std::cout << v[0] << ' ' << v[1] << ' ' << s.size() << '\n';
}
