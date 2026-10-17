#include <cstdio>
//%%
namespace { int hidden = 3; }
//%%
namespace { int get() { return hidden * 2; } }
//%%
namespace { struct P { int a = 4; }; }
//%%
int use() { return get() + hidden + P{}.a; }
//%%
namespace { int hidden = 9; }
//%%
namespace { int more = hidden + 1; }
//%%
static int counter;
//%%
int bump() { return ++counter; }
//%%
int main() { bump(); std::printf("%d %d %d\n", use(), more, bump()); }
