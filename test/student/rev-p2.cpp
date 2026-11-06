#include <iostream>
#include <vector>
#include <string>
#include <string_view>
#include <map>
#include <memory>
#include <algorithm>
#include <list>
using namespace std;
struct Node { int v; Node *left = nullptr, *right = nullptr; Node(int v) : v(v) {} };
Node *insert(Node *r, int v) { if (!r) return new Node(v); if (v < r->v) r->left = insert(r->left, v); else r->right = insert(r->right, v); return r; }
void free_tree(Node *r) { if (!r) return; free_tree(r->left); free_tree(r->right); delete r; }
struct List { struct N { int v; N *next; }; N *head = nullptr;
  void add(int v) { head = new N{v, head}; }
  ~List() { while (head) { N *t = head; head = head->next; delete t; } } };
const string &get_first(const vector<string> &v) { return v.front(); }
string_view prefix(string_view s, size_t n) { return s.substr(0, n); }
class PhoneBook {
  map<string, string> phones;
public:
  void add(const string &n, const string &t) { phones[n] = t; }
  const string *lookup(const string &n) const { auto it = phones.find(n); return it == phones.end() ? nullptr : &it->second; }
  string &operator[](const string &n) { return phones[n]; }
};
int main() {
  vector<int> v = {3, 1, 2};
  v.reserve(100);
  int &a = v[0]; a = 5;
  for (int &x : v) x *= 2;
  auto it = v.begin();
  while (it != v.end()) { if (*it == 2) it = v.erase(it); else ++it; }
  v.push_back(7);
  int &b = v.back(); b++;
  vector<string> names = {"ana", "bia"};
  const string &p = get_first(names); cout << p;
  string s = "hello world";
  string_view pv = prefix(s, 5); cout << pv;
  PhoneBook ag; ag.add("a", "1");
  if (const string *t = ag.lookup("a")) cout << *t;
  ag["b"] = "2";
  string &ref = ag["c"]; ref = "3";
  Node *r = nullptr; for (int x : {5, 3, 8}) r = insert(r, x); free_tree(r);
  List l; l.add(1); l.add(2);
  unique_ptr<int> up = make_unique<int>(4); int *raw = up.get(); cout << *raw;
  int *arr = new int[10]; for (int i = 0; i < 10; i++) arr[i] = i; int *last = arr + 10; cout << *(last - 1); delete[] arr;
  list<int> li = {1, 2, 3}; auto lit = li.begin(); li.push_back(4); cout << *lit;
  map<string, int> m; int &cnt = m["x"]; m["y"] = 2; cnt++;
  string &first = names[0]; first += "!"; cout << first;
  const char *c = s.c_str(); cout << c;
  int *q = nullptr; { static int y = 3; q = &y; } cout << *q;
  vector<vector<int>> g(3); for (auto &row : g) row.push_back(1);
  auto &row0 = g[0]; row0.push_back(2);
  sort(v.begin(), v.end(), [](int x, int y) { return x > y; });
  string t = s; t.append("x"); string_view tv = t; cout << tv;
  int *pp = new int(5); int *alias = pp; delete pp; pp = nullptr; alias = nullptr;
  cout << endl;
}
