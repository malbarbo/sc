#include <algorithm>
#include <iostream>
#include <vector>

int main() {
    std::vector<int> v = {5, 3, 8, 3, 1, 9, 2, 3};
    std::sort(v.begin(), v.end(), [](int a, int b) { return a <= b; });
    for (int x : v)
        std::cout << x << ' ';
    std::cout << '\n';
}
