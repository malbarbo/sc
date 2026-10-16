#include <iostream>
#include <vector>

int main() {
    std::vector<int> v = {1, 2, 3};
    int &primeiro = v[0];
    for (int i = 0; i < 10; i++) {
        v.push_back(i);
    }
    std::cout << primeiro << "\n";
}
