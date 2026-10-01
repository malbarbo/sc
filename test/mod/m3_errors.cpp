// compile errors through import std: the diagnostics must match
import std;
int main() {
  std::vector<int> v;
  v.push_back("x");
  std::cout << v << '\n';
  std::map<int, int> m;
  m.insert(1);
  std::string s = 5;
  std::sort(m.begin(), m.end());
  return std::undefined_thing;
}
