#include <iostream>
#include <vector>
#include <string>
#include <string_view>
#include <map>
#include <memory>
#include <algorithm>
#include <list>
using namespace std;
struct No { int v; No *esq = nullptr, *dir = nullptr; No(int v) : v(v) {} };
No *insere(No *r, int v) { if (!r) return new No(v); if (v < r->v) r->esq = insere(r->esq, v); else r->dir = insere(r->dir, v); return r; }
void libera(No *r) { if (!r) return; libera(r->esq); libera(r->dir); delete r; }
struct Lista { struct N { int v; N *prox; }; N *ini = nullptr;
  void add(int v) { ini = new N{v, ini}; }
  ~Lista() { while (ini) { N *t = ini; ini = ini->prox; delete t; } } };
const string &primeiro(const vector<string> &v) { return v.front(); }
string_view prefixo(string_view s, size_t n) { return s.substr(0, n); }
class Agenda {
  map<string, string> tel;
public:
  void add(const string &n, const string &t) { tel[n] = t; }
  const string *busca(const string &n) const { auto it = tel.find(n); return it == tel.end() ? nullptr : &it->second; }
  string &operator[](const string &n) { return tel[n]; }
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
  vector<string> nomes = {"ana", "bia"};
  const string &p = primeiro(nomes); cout << p;
  string s = "hello world";
  string_view pv = prefixo(s, 5); cout << pv;
  Agenda ag; ag.add("a", "1");
  if (const string *t = ag.busca("a")) cout << *t;
  ag["b"] = "2";
  string &ref = ag["c"]; ref = "3";
  No *r = nullptr; for (int x : {5, 3, 8}) r = insere(r, x); libera(r);
  Lista l; l.add(1); l.add(2);
  unique_ptr<int> up = make_unique<int>(4); int *raw = up.get(); cout << *raw;
  int *arr = new int[10]; for (int i = 0; i < 10; i++) arr[i] = i; int *fim = arr + 10; cout << *(fim - 1); delete[] arr;
  list<int> li = {1, 2, 3}; auto lit = li.begin(); li.push_back(4); cout << *lit;
  map<string, int> m; int &cnt = m["x"]; m["y"] = 2; cnt++;
  string &first = nomes[0]; first += "!"; cout << first;
  const char *c = s.c_str(); cout << c;
  int *q = nullptr; { static int y = 3; q = &y; } cout << *q;
  vector<vector<int>> g(3); for (auto &row : g) row.push_back(1);
  auto &row0 = g[0]; row0.push_back(2);
  sort(v.begin(), v.end(), [](int x, int y) { return x > y; });
  string t = s; t.append("x"); string_view tv = t; cout << tv;
  int *pp = new int(5); int *alias = pp; delete pp; pp = nullptr; alias = nullptr;
  cout << endl;
}
