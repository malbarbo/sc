#include <vector>
//%%
std::vector<int> v{1, 2, 3};
//%%
import std;
//%%
int sum() { int s = 0; for (int x : v) s += x; return s; }
//%%
int main() { std::println("{} {}", sum(), std::ranges::max(v)); }
