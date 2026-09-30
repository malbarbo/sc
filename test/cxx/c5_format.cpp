// iomanip, stringstream, numeric conversions, cmath
#include <cmath>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>
int main() {
  std::cout << std::fixed << std::setprecision(3) << M_PI << ' ' << std::sqrt(2.0) << '\n';
  std::cout << std::setw(8) << std::right << 42 << '|' << std::left << std::setw(6) << "ab" << '|' << '\n';
  std::cout << std::hex << 255 << ' ' << std::dec << 255 << ' ' << std::scientific << 12345.678 << '\n';
  std::stringstream ss("10 20 30 abc");
  int x, t = 0;
  while (ss >> x) t += x;
  std::cout << t << '\n';
  std::ostringstream os;
  os << "n=" << 7 << ", f=" << 1.25;
  std::cout << os.str() << ' ' << std::stoi("123") + 1 << ' ' << std::to_string(3.5) << ' ' << std::stod("2.5") * 2 << '\n';
  std::vector<std::vector<int>> m(3, std::vector<int>(3));
  for (int i = 0; i < 3; ++i)
    for (int j = 0; j < 3; ++j) m[i][j] = i * 3 + j;
  for (auto &r : m) { for (int e : r) std::cout << std::setw(3) << e; std::cout << '\n'; }
  std::string w;
  int words = 0;
  while (std::cin >> w) words++;
  std::cout << "words " << words << '\n';
}
