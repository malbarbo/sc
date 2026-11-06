#include <vector>
#include <string>
using namespace std;
int main() {
  vector<vector<int>> g(3);
  g[0].push_back(1);
  for (auto &row : g) row.push_back(1);
  auto &row0 = g[1]; row0.push_back(2);
  vector<string> names = {"ana"};
  string &f = names[0]; f += "!";
  names[0] += "?";
  vector<int> adj[5]; adj[0].push_back(1);
  return (int) g.size();
}
