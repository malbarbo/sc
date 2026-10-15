#include <iostream>

struct Turma {
    int soma = 0, n = 0;
    int media() const { return soma / n; }
};

int main() {
    Turma t;
    std::cout << t.media() << '\n';
}
