// stack template over a vector, operator overloading and a struct
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
using namespace std;

template <typename T>
class Stack {
    vector<T> items;

public:
    void push(const T &x) { items.push_back(x); }
    T pop() {
        if (items.empty()) throw out_of_range("empty stack");
        T x = items.back();
        items.pop_back();
        return x;
    }
    bool empty() const { return items.empty(); }
    size_t size() const { return items.size(); }
};

struct Fraction {
    int num, den;
};

Fraction operator+(Fraction a, Fraction b) {
    return {a.num * b.den + b.num * a.den, a.den * b.den};
}

ostream &operator<<(ostream &os, const Fraction &f) {
    return os << f.num << '/' << f.den;
}

bool balanced(const string &s) {
    Stack<char> p;
    for (char c : s) {
        if (c == '(' || c == '[') {
            p.push(c);
        } else if (c == ')' || c == ']') {
            if (p.empty()) return false;
            char a = p.pop();
            if ((c == ')' && a != '(') || (c == ']' && a != '[')) return false;
        }
    }
    return p.empty();
}

int main() {
    Stack<int> p;
    for (int i = 1; i <= 3; i++) p.push(i * 10);
    while (!p.empty()) cout << p.pop() << ' ';
    cout << '\n';
    cout << boolalpha << balanced("([()])") << ' ' << balanced("(]") << '\n';
    cout << Fraction{1, 2} + Fraction{1, 3} << '\n';
    try {
        p.pop();
    } catch (const out_of_range &e) {
        cout << e.what() << '\n';
    }
}
