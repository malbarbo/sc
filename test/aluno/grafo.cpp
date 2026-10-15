// grafo: lista de adjacência, busca em largura e caminho mínimo
#include <iostream>
#include <queue>
#include <vector>
using namespace std;

vector<int> bfs(const vector<vector<int>> &adj, int origem) {
    vector<int> dist(adj.size(), -1);
    queue<int> fila;
    dist[origem] = 0;
    fila.push(origem);
    while (!fila.empty()) {
        int u = fila.front();
        fila.pop();
        for (int v : adj[u]) {
            if (dist[v] == -1) {
                dist[v] = dist[u] + 1;
                fila.push(v);
            }
        }
    }
    return dist;
}

int main() {
    int n = 6;
    vector<vector<int>> adj(n);
    int arestas[][2] = {{0, 1}, {0, 2}, {1, 3}, {2, 3}, {3, 4}};
    for (auto &a : arestas) {
        adj[a[0]].push_back(a[1]);
        adj[a[1]].push_back(a[0]);
    }
    vector<int> d = bfs(adj, 0);
    for (int i = 0; i < n; i++) {
        cout << i << ": " << d[i] << '\n';
    }
}
