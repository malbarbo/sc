#include <iostream>
#include <optional>

std::optional<int> busca(int x) {
    if (x > 0) {
        return x;
    }
    return std::nullopt;
}

int main() {
    std::cout << *busca(-1) << '\n';
}
