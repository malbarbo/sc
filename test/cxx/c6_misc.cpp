// enum class, namespaces, static members, references, new/delete, structs in
// structs, recursion, C library from C++
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
namespace geo {
enum class Color { Red, Green, Blue };
const char *name(Color c) {
  switch (c) { case Color::Red: return "red"; case Color::Green: return "green"; default: return "blue"; }
}
} // namespace geo
struct Counter {
  static int created;
  int id;
  Counter() : id(++created) {}
  Counter(const Counter &o) : id(o.id + 100) { ++created; }
  ~Counter() { std::printf("bye %d\n", id); }
};
int Counter::created = 0;
struct Node { int val; Node *next; };
void swapRef(int &a, int &b) { int t = a; a = b; b = t; }
int hanoi(int n) { return n == 0 ? 0 : 2 * hanoi(n - 1) + 1; }
class Account {
  double balance = 0;
public:
  void deposit(double v) { if (v > 0) balance += v; }
  double get() const { return balance; }
};
int main() {
  std::cout << geo::name(geo::Color::Green) << '\n';
  { Counter a, b; Counter c = a; std::cout << Counter::created << ' ' << c.id << '\n'; }
  Node *head = nullptr;
  for (int i = 0; i < 5; ++i) head = new Node{i, head};
  int s = 0;
  while (head) { s += head->val; Node *n = head->next; delete head; head = n; }
  int *arr = new int[10];
  for (int i = 0; i < 10; ++i) arr[i] = i * i;
  std::cout << s << ' ' << arr[9] << '\n';
  delete[] arr;
  int x = 1, y = 2;
  swapRef(x, y);
  char buf[32];
  std::snprintf(buf, sizeof buf, "%d-%d", x, y);
  std::cout << buf << ' ' << std::strlen(buf) << ' ' << hanoi(10) << ' ' << std::abs(-5) << '\n';
  Account acc;
  acc.deposit(10.5);
  acc.deposit(-3);
  std::cout << acc.get() << '\n';
  auto p = std::string("a") + 'b' + "c";
  std::cout << p << '\n';
  return 0;
}
