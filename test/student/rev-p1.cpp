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

class Point {
    double x, y;
public:
    Point(double x, double y) : x(x), y(y) {}
    double getX() const { return x; }
    double distance(const Point &o) const { return sqrt((x - o.x) * (x - o.x) + (y - o.y) * (y - o.y)); }
    void setX(double x) { this->x = x; }
};

class Shape {
public:
    virtual double area() const = 0;
    virtual ~Shape() = default;
};
class Circle : public Shape {
    double r;
public:
    explicit Circle(double r) : r(r) {}
    double area() const override { return M_PI * r * r; }
};
class Rectangle : public Shape {
    double l, a;
public:
    Rectangle(double l, double a) : l(l), a(a) {}
    double area() const override { return l * a; }
};

struct Student { string name; int age; double grade; };

class Account {
    string owner;
    double balance = 0;
public:
    Account(string owner) : owner(std::move(owner)) {}
    void deposit(double v) { if (v <= 0) throw invalid_argument("amount"); balance += v; }
    double getBalance() const { return balance; }
    const string &getOwner() const { return owner; }
};

class Stack {
    vector<int> items;
public:
    void push(int v) { items.push_back(v); }
    int pop() { int v = items.back(); items.pop_back(); return v; }
    bool is_empty() const { return items.empty(); }
    int top() const { return items.back(); }
};

template <typename T>
T largest(const vector<T> &v) {
    T m = v[0];
    for (const auto &x : v) if (x > m) m = x;
    return m;
}

int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }

string to_upper(string s) {
    for (char &c : s) c = toupper(c);
    return s;
}

vector<string> split(const string &s, char sep) {
    vector<string> r;
    stringstream ss(s);
    string item;
    while (getline(ss, item, sep)) r.push_back(item);
    return r;
}

const string &longer(const string &a, const string &b) { return a.size() > b.size() ? a : b; }

optional<int> find_index(const vector<int> &v, int x) {
    for (size_t i = 0; i < v.size(); i++) if (v[i] == x) return i;
    return nullopt;
}

int main() {
    vector<int> v = {5, 3, 8, 1};
    sort(v.begin(), v.end());
    for (int i = 0; i < v.size(); i++) cout << v[i] << ' ';
    cout << '\n';
    for (auto x : v) cout << x << ' ';
    cout << largest(v) << ' ' << accumulate(v.begin(), v.end(), 0) << '\n';
    map<string, int> freq;
    string word;
    istringstream in("a b a c b a");
    while (in >> word) freq[word]++;
    for (const auto &[k, c] : freq) cout << k << ": " << c << '\n';
    for (auto it = freq.begin(); it != freq.end(); ++it) cout << it->first << it->second;
    vector<unique_ptr<Shape>> shapes;
    shapes.push_back(make_unique<Circle>(1.0));
    shapes.push_back(make_unique<Rectangle>(2, 3));
    double total = 0;
    for (const auto &f : shapes) total += f->area();
    cout << total << '\n';
    Point p(1, 2), q(4, 6);
    cout << p.distance(q) << '\n';
    vector<Student> students = {{"Ana", 20, 9.5}, {"Bia", 21, 7.0}};
    sort(students.begin(), students.end(), [](const Student &a, const Student &b) { return a.grade > b.grade; });
    for (const Student &a : students) cout << a.name << '\n';
    Account c("Ze");
    try { c.deposit(-1); } catch (const exception &e) { cout << e.what() << '\n'; }
    Stack stk; stk.push(1); stk.push(2);
    while (!stk.is_empty()) cout << stk.pop();
    string s = "hello";
    cout << to_upper(s) << s.length() << s.substr(1, 3) << '\n';
    for (const auto &w : split("a,b,c", ',')) cout << w;
    const string &ml = longer(s, "xy");
    cout << ml << '\n';
    string a = "abc", b = "de";
    const string &ml2 = longer(a, b);
    cout << ml2 << '\n';
    if (auto r = find_index(v, 3)) cout << *r << '\n';
    vector<vector<int>> mat(3, vector<int>(3, 0));
    for (size_t i = 0; i < mat.size(); i++) mat[i][i] = 1;
    set<int> st(v.begin(), v.end());
    cout << st.count(3) << fib(10) << '\n';
    int n = v.size();
    double mean = (double) accumulate(v.begin(), v.end(), 0) / n;
    int rounded = round(mean);
    cout << rounded << '\n';
    auto it = find(v.begin(), v.end(), 8);
    if (it != v.end()) v.erase(it);
    vector<int> w;
    w.reserve(10);
    for (int i = 0; i < 10; i++) w.push_back(i);
    int &ref = w[0];
    ref = 3;
    pair<int, string> pr = {1, "one"};
    cout << pr.first << pr.second << '\n';
    string_view sv = s;
    cout << sv << '\n';
    string name;
    getline(cin, name);
    char ch = s[0];
    cout << ch << (int) 'a' << '\n';
    return 0;
}
