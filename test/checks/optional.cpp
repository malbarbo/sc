#include <iostream>
#include <optional>

std::optional<int> lookup(int x) {
    if (x > 0) {
        return x;
    }
    return std::nullopt;
}

int main() {
    std::cout << *lookup(-1) << '\n';
}
