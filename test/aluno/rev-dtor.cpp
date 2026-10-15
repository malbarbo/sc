#include <iostream>
#include <vector>
#include <string>
using namespace std;
class Obj {
  string nome;
public:
  Obj(string n) : nome(n) { cout << "cria " << nome << '\n'; }
  ~Obj() { cout << "destroi " << nome << '\n'; }
};
void usa(Obj o) { (void) o; }
int main() {
  Obj a("a");
  Obj b = a;
  usa(a);
  vector<Obj> v; v.push_back(a);
}
