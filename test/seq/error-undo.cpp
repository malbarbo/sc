#include <cstdio>
//%%
struct A { int v; };
//%%
//%undo
//%%
struct A { int v; int w; };
//%%
int k = A{1, 2}.w;
//%%
template <class T> T twice(T x) { return x + x; }
//%%
int bad = twice("x");
//%%
int good = twice(k);
//%%
int main() { std::printf("%d\n", good); }
