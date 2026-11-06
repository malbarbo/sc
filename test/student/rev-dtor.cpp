#include <iostream>
#include <vector>
#include <string>
using namespace std;
class Obj {
  string name;
public:
  Obj(string n) : name(n) { cout << "create " << name << '\n'; }
  ~Obj() { cout << "destroy " << name << '\n'; }
};
void use(Obj o) { (void) o; }
int main() {
  Obj a("a");
  Obj b = a;
  use(a);
  vector<Obj> v; v.push_back(a);
}
