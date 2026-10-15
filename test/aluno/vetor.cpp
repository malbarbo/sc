// vector: leitura, ordenação, média, remoção e busca
#include <algorithm>
#include <iostream>
#include <numeric>
#include <vector>
using namespace std;

double media(const vector<int> &v) {
    if (v.empty()) return 0;
    return accumulate(v.begin(), v.end(), 0.0) / v.size();
}

int main() {
    vector<int> v = {7, 3, 9, 1, 4, 9, 2};
    sort(v.begin(), v.end());
    for (size_t i = 0; i < v.size(); i++) {
        cout << v[i] << (i + 1 < v.size() ? " " : "\n");
    }
    cout << "media: " << media(v) << '\n';
    v.erase(unique(v.begin(), v.end()), v.end());
    v.erase(remove_if(v.begin(), v.end(), [](int x) { return x % 2 == 0; }), v.end());
    for (int x : v) cout << x << ' ';
    cout << '\n';
    auto it = find(v.begin(), v.end(), 9);
    if (it != v.end()) cout << "9 na posicao " << it - v.begin() << '\n';
    vector<vector<int>> m(3, vector<int>(3, 0));
    for (int i = 0; i < 3; i++) m[i][i] = 1;
    for (const auto &linha : m) {
        for (int x : linha) cout << x;
        cout << '\n';
    }
    int maior = *max_element(v.begin(), v.end());
    cout << "maior: " << maior << '\n';
}
