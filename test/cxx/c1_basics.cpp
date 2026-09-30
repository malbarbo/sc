// iostream, strings, vectors, loops, getline from stdin
#include <iostream>
#include <string>
#include <vector>
using namespace std;
int main() {
  string line;
  vector<string> lines;
  while (getline(cin, line)) lines.push_back(line);
  cout << "lines: " << lines.size() << endl;
  for (size_t i = 0; i < lines.size(); ++i) cout << i << ": " << lines[i] << " (" << lines[i].size() << ")\n";
  int sum = 0;
  for (int i = 1; i <= 100; ++i) sum += i;
  cout << "sum " << sum << ' ' << 3.5 * 2 << ' ' << true << ' ' << 'x' << '\n';
  string s = "hello";
  s += ", world";
  cout << s.substr(0, 5) << '|' << s.find("world") << '|' << (s < "help") << '\n';
}
