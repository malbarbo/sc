// Sorteia a ordem de apresentação dos grupos e conta as letras dos nomes
#include <algorithm>
#include <iostream>
#include <random>
#include <string>
#include <unordered_map>
#include <vector>

int main() {
    std::vector<std::string> grupos = {"Ana", "Bruno", "Carla", "Davi", "Elisa", "Fábio"};
    std::mt19937 gerador(2026);
    std::shuffle(grupos.begin(), grupos.end(), gerador);
    std::uniform_int_distribution<int> dado(1, 6);
    std::unordered_map<char, int> letras;
    for (const std::string &g : grupos) {
        for (char c : g)
            letras[c]++;
        std::cout << g << ' ' << dado(gerador) << '\n';
    }
    std::cout << "letras diferentes: " << letras.size() << '\n';
}
