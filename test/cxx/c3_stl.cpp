// containers and algorithms
#include <algorithm>
#include <deque>
#include <iostream>
#include <list>
#include <map>
#include <numeric>
#include <queue>
#include <set>
#include <stack>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>
int main() {
  std::vector<int> v{5, 3, 9, 1, 7, 3};
  std::sort(v.begin(), v.end());
  v.erase(std::unique(v.begin(), v.end()), v.end());
  for (int x : v) std::cout << x << ' ';
  std::cout << "\nsum " << std::accumulate(v.begin(), v.end(), 0) << " max " << *std::max_element(v.begin(), v.end()) << '\n';
  std::cout << std::binary_search(v.begin(), v.end(), 7) << ' ' << std::count_if(v.begin(), v.end(), [](int x) { return x > 4; }) << '\n';
  std::map<std::string, int> freq;
  for (auto w : {"a", "b", "a", "c", "b", "a"}) freq[w]++;
  for (auto &[k, n] : freq) std::cout << k << '=' << n << ' ';
  std::cout << '\n';
  std::unordered_map<int, std::string> um{{1, "one"}, {2, "two"}};
  std::cout << um[2] << ' ' << um.count(3) << '\n';
  std::set<int> s(v.begin(), v.end());
  std::unordered_set<std::string> us{"x", "y"};
  std::cout << s.size() << ' ' << us.count("x") << '\n';
  std::stack<int> st; std::queue<int> q; std::deque<int> dq; std::list<int> l;
  std::priority_queue<int> pq;
  for (int i = 0; i < 5; ++i) { st.push(i); q.push(i); dq.push_front(i); l.push_back(i * i); pq.push((i * 7) % 5); }
  std::cout << st.top() << q.front() << dq.front() << l.back() << pq.top() << '\n';
  std::reverse(v.begin(), v.end());
  std::vector<int> w(v.size());
  std::transform(v.begin(), v.end(), w.begin(), [](int x) { return x * 2; });
  for (int x : w) std::cout << x << ' ';
  std::cout << '\n';
  std::vector<std::pair<std::string, int>> ps{{"bob", 3}, {"al", 5}, {"cy", 3}};
  std::stable_sort(ps.begin(), ps.end(), [](auto &a, auto &b) { return a.second < b.second; });
  for (auto &p : ps) std::cout << p.first << ' ';
  std::cout << '\n';
}
