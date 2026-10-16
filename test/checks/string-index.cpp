#include <iostream>
#include <string>

int main() {
    std::string s = "abc";
    for (std::size_t i = 0; i <= s.size() + 1; i++) {
        std::cout << s[i];
    }
    std::cout << '\n';
}
