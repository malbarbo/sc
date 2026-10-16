#include <iostream>
#include <map>
#include <string>
#include <vector>

struct Pilha {
    int *itens;
    int topo = 0;
    Pilha() : itens(new int[10]) {}
    void empilha(int x) { itens[topo++] = x; }
};

int main() {
    std::map<std::string, int> conta;
    for (std::string p : {"a", "b", "a"}) {
        conta[p]++;
    }
    std::vector<Pilha> pilhas;
    for (int i = 0; i < 2; i++) {
        pilhas.push_back(Pilha());
        pilhas.back().empilha(i);
    }
    std::cout << conta["a"] << " " << pilhas.size() << "\n";
}
