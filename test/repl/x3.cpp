// entry 3: import std, on top of entries compiled with #include
import std;
extern std::vector<std::string> v;
int main() {
  v.push_back("three");
  for (const auto &s : v) std::cout << std::format("[{}]", s);
  std::map<std::string, int> m{{"a", 1}};
  std::cout << " m[a] = " << m["a"] << std::endl;
}
