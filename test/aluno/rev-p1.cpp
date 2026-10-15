#include <iostream>
#include <vector>
#include <string>
#include <map>
#include <algorithm>
#include <memory>
#include <cmath>
#include <sstream>
#include <set>
#include <stdexcept>
#include <numeric>
#include <utility>
#include <optional>
#include <string_view>
using namespace std;

class Ponto {
    double x, y;
public:
    Ponto(double x, double y) : x(x), y(y) {}
    double getX() const { return x; }
    double distancia(const Ponto &o) const { return sqrt((x - o.x) * (x - o.x) + (y - o.y) * (y - o.y)); }
    void setX(double x) { this->x = x; }
};

class Forma {
public:
    virtual double area() const = 0;
    virtual ~Forma() = default;
};
class Circulo : public Forma {
    double r;
public:
    explicit Circulo(double r) : r(r) {}
    double area() const override { return M_PI * r * r; }
};
class Retangulo : public Forma {
    double l, a;
public:
    Retangulo(double l, double a) : l(l), a(a) {}
    double area() const override { return l * a; }
};

struct Aluno { string nome; int idade; double nota; };

class Conta {
    string titular;
    double saldo = 0;
public:
    Conta(string titular) : titular(std::move(titular)) {}
    void deposita(double v) { if (v <= 0) throw invalid_argument("valor"); saldo += v; }
    double getSaldo() const { return saldo; }
    const string &getTitular() const { return titular; }
};

class Pilha {
    vector<int> dados;
public:
    void push(int v) { dados.push_back(v); }
    int pop() { int v = dados.back(); dados.pop_back(); return v; }
    bool vazia() const { return dados.empty(); }
    int topo() const { return dados.back(); }
};

template <typename T>
T maior(const vector<T> &v) {
    T m = v[0];
    for (const auto &x : v) if (x > m) m = x;
    return m;
}

int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }

string maiuscula(string s) {
    for (char &c : s) c = toupper(c);
    return s;
}

vector<string> divide(const string &s, char sep) {
    vector<string> r;
    stringstream ss(s);
    string item;
    while (getline(ss, item, sep)) r.push_back(item);
    return r;
}

const string &maisLongo(const string &a, const string &b) { return a.size() > b.size() ? a : b; }

optional<int> acha(const vector<int> &v, int x) {
    for (size_t i = 0; i < v.size(); i++) if (v[i] == x) return i;
    return nullopt;
}

int main() {
    vector<int> v = {5, 3, 8, 1};
    sort(v.begin(), v.end());
    for (int i = 0; i < v.size(); i++) cout << v[i] << ' ';
    cout << '\n';
    for (auto x : v) cout << x << ' ';
    cout << maior(v) << ' ' << accumulate(v.begin(), v.end(), 0) << '\n';
    map<string, int> freq;
    string palavra;
    istringstream in("a b a c b a");
    while (in >> palavra) freq[palavra]++;
    for (const auto &[k, c] : freq) cout << k << ": " << c << '\n';
    for (auto it = freq.begin(); it != freq.end(); ++it) cout << it->first << it->second;
    vector<unique_ptr<Forma>> formas;
    formas.push_back(make_unique<Circulo>(1.0));
    formas.push_back(make_unique<Retangulo>(2, 3));
    double total = 0;
    for (const auto &f : formas) total += f->area();
    cout << total << '\n';
    Ponto p(1, 2), q(4, 6);
    cout << p.distancia(q) << '\n';
    vector<Aluno> turma = {{"Ana", 20, 9.5}, {"Bia", 21, 7.0}};
    sort(turma.begin(), turma.end(), [](const Aluno &a, const Aluno &b) { return a.nota > b.nota; });
    for (const Aluno &a : turma) cout << a.nome << '\n';
    Conta c("Ze");
    try { c.deposita(-1); } catch (const exception &e) { cout << e.what() << '\n'; }
    Pilha pl; pl.push(1); pl.push(2);
    while (!pl.vazia()) cout << pl.pop();
    string s = "hello";
    cout << maiuscula(s) << s.length() << s.substr(1, 3) << '\n';
    for (const auto &w : divide("a,b,c", ',')) cout << w;
    const string &ml = maisLongo(s, "xy");
    cout << ml << '\n';
    string a = "abc", b = "de";
    const string &ml2 = maisLongo(a, b);
    cout << ml2 << '\n';
    if (auto r = acha(v, 3)) cout << *r << '\n';
    vector<vector<int>> mat(3, vector<int>(3, 0));
    for (size_t i = 0; i < mat.size(); i++) mat[i][i] = 1;
    set<int> st(v.begin(), v.end());
    cout << st.count(3) << fib(10) << '\n';
    int n = v.size();
    double media = (double) accumulate(v.begin(), v.end(), 0) / n;
    int arred = round(media);
    cout << arred << '\n';
    auto it = find(v.begin(), v.end(), 8);
    if (it != v.end()) v.erase(it);
    vector<int> w;
    w.reserve(10);
    for (int i = 0; i < 10; i++) w.push_back(i);
    int &ref = w[0];
    ref = 3;
    pair<int, string> pr = {1, "um"};
    cout << pr.first << pr.second << '\n';
    string_view sv = s;
    cout << sv << '\n';
    string nome;
    getline(cin, nome);
    char ch = s[0];
    cout << ch << (int) 'a' << '\n';
    return 0;
}
