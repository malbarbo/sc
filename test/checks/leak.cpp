#include <iostream>
#include <map>
#include <string>
#include <vector>

struct Stack {
    int *items;
    int top = 0;
    Stack() : items(new int[10]) {}
    void push(int x) { items[top++] = x; }
};

int main() {
    std::map<std::string, int> counts;
    for (std::string p : {"a", "b", "a"}) {
        counts[p]++;
    }
    std::vector<Stack> stacks;
    for (int i = 0; i < 2; i++) {
        stacks.push_back(Stack());
        stacks.back().push(i);
    }
    std::cout << counts["a"] << " " << stacks.size() << "\n";
}
