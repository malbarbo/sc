// import std: iostream, strings, vectors, sort, map, getline from stdin
import std;
using namespace std;
int main() {
  string line;
  vector<string> lines;
  while (getline(cin, line)) lines.push_back(line);
  cout << "lines: " << lines.size() << endl;
  sort(lines.begin(), lines.end(), greater<>());
  map<string, int> len;
  for (auto &l : lines) len[l] = int(l.size());
  for (auto &[k, v] : len) cout << k << " -> " << v << '\n';
  cout << 3.5 * 2 << ' ' << boolalpha << true << ' ' << setw(5) << 42 << '\n';
}
