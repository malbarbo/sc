#include <iostream>

struct Classroom {
    int sum = 0, n = 0;
    int average() const { return sum / n; }
};

int main() {
    Classroom t;
    std::cout << t.average() << '\n';
}
