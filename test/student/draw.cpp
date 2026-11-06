// Draws the order in which the groups present and counts the letters of the names
#include <algorithm>
#include <iostream>
#include <random>
#include <string>
#include <unordered_map>
#include <vector>

int main() {
    std::vector<std::string> groups = {"Ana", "Bruno", "Carla", "Davi", "Elisa", "Fábio"};
    std::mt19937 generator(2026);
    std::shuffle(groups.begin(), groups.end(), generator);
    std::uniform_int_distribution<int> die(1, 6);
    std::unordered_map<char, int> letters;
    for (const std::string &g : groups) {
        for (char c : g)
            letters[c]++;
        std::cout << g << ' ' << die(generator) << '\n';
    }
    std::cout << "distinct letters: " << letters.size() << '\n';
}
