// entry 2: the same inline function and classes, from the same "header"
#include <vector>
#include <string>
#include <cstdio>
extern std::vector<std::string> v;
struct Base { virtual ~Base() {} virtual int id() const { return 1; } };
struct Der : Base { int id() const override { return 2; } };
extern Base *obj;
inline int &counter() { static int c = 0; return c; }
int tick1();
__attribute__((constructor)) static void run() {
  v.push_back("dois");
  tick1(); tick1(); ++counter();
  std::printf("v.size = %zu, counter = %d, id = %d, dynamic_cast: %s\n", v.size(), counter(), obj->id(),
              dynamic_cast<Der *>(obj) ? "ok" : "FALHOU");
}
