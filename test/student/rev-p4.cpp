#include <iostream>
#include <string>
#include <vector>
#include <memory>
using namespace std;
struct N { int v; N *next; };
struct L { N *head = nullptr; void remove(int x) {
  N **pp = &head; while (*pp) { if ((*pp)->v == x) { N *t = *pp; *pp = t->next; delete t; } else pp = &(*pp)->next; } } };
int main() {
  int *p = new int(1); delete p; p = new int(2); cout << *p; delete p;
  N *a = new N{1, nullptr}; N *b = new N{2, a}; N *cur = b;
  while (cur) { N *nx = cur->next; delete cur; cur = nx; }
  const int *m = nullptr; int best = -1;
  for (int i = 0; i < 3; i++) { int val = i * 2; if (val > best) { best = val; m = &best; } }
  cout << *m;
  string r; for (int i = 0; i < 3; i++) { string t = to_string(i); r += t; } cout << r;
  const string *ps; string s1 = "a", s2 = "b"; ps = s1 < s2 ? &s1 : &s2; cout << *ps;
  vector<int> v(3); int *data = v.data(); data[0] = 1; cout << v[0];
  char buf[10]; char *w = buf; { *w++ = 'a'; } *w = 0; cout << buf;
  unique_ptr<N> u(new N{3, nullptr}); N *raw = u.get(); u.reset(); raw = nullptr;
  int *arr = new int[3]; int *q = arr; delete[] arr; arr = new int[3]; q = arr; q[0] = 1; delete[] arr;
  return 0;
}
