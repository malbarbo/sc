// template de pilha com vector, sobrecarga de operador e struct
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
using namespace std;

template <typename T>
class Pilha {
    vector<T> itens;

public:
    void empilha(const T &x) { itens.push_back(x); }
    T desempilha() {
        if (itens.empty()) throw out_of_range("pilha vazia");
        T x = itens.back();
        itens.pop_back();
        return x;
    }
    bool vazia() const { return itens.empty(); }
    size_t tamanho() const { return itens.size(); }
};

struct Fracao {
    int num, den;
};

Fracao operator+(Fracao a, Fracao b) {
    return {a.num * b.den + b.num * a.den, a.den * b.den};
}

ostream &operator<<(ostream &os, const Fracao &f) {
    return os << f.num << '/' << f.den;
}

bool balanceado(const string &s) {
    Pilha<char> p;
    for (char c : s) {
        if (c == '(' || c == '[') {
            p.empilha(c);
        } else if (c == ')' || c == ']') {
            if (p.vazia()) return false;
            char a = p.desempilha();
            if ((c == ')' && a != '(') || (c == ']' && a != '[')) return false;
        }
    }
    return p.vazia();
}

int main() {
    Pilha<int> p;
    for (int i = 1; i <= 3; i++) p.empilha(i * 10);
    while (!p.vazia()) cout << p.desempilha() << ' ';
    cout << '\n';
    cout << boolalpha << balanceado("([()])") << ' ' << balanceado("(]") << '\n';
    cout << Fracao{1, 2} + Fracao{1, 3} << '\n';
    try {
        p.desempilha();
    } catch (const out_of_range &e) {
        cout << e.what() << '\n';
    }
}
