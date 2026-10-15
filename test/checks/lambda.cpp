#include <algorithm>
#include <iostream>
#include <vector>

int main() {
    std::vector<int> v{3, 0, 1};
    std::sort(v.begin(), v.end(), [](int a, int b) { return 10 / a < 10 / b; });
    std::cout << v[0] << '\n';
}
