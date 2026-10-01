// entry 1: a vector that lives across entries and a class with inline virtuals
#include <vector>
#include <string>
std::vector<std::string> v{"um"};
struct Base { virtual ~Base() {} virtual int id() const { return 1; } };
struct Der : Base { int id() const override { return 2; } };
Base *obj = new Der;
inline int &counter() { static int c = 0; return c; }
int tick1() { return ++counter(); }
