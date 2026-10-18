#include <cstdio>
#include <utility>
//%%
auto [a, b] = std::pair{1, 2};
int bad = nope;
//%%
auto [a, b] = std::pair{3, 4};
//%%
int main() { std::printf("%d %d\n", a, b); }
