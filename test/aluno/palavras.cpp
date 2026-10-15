// string e map: contagem de palavras lidas da entrada
#include <cctype>
#include <iostream>
#include <map>
#include <sstream>
#include <string>
using namespace std;

string minusculas(string s) {
    for (char &c : s) c = tolower(c);
    return s;
}

int main() {
    map<string, int> conta;
    string linha;
    int linhas = 0;
    while (getline(cin, linha)) {
        linhas++;
        istringstream in(linha);
        string palavra;
        while (in >> palavra) conta[minusculas(palavra)]++;
    }
    cout << linhas << " linhas\n";
    for (const auto &[palavra, n] : conta) cout << palavra << ": " << n << '\n';
    string nome = "Maria da Silva";
    auto pos = nome.find(' ');
    cout << nome.substr(0, pos) << " / " << nome.size() << '\n';
    string invertida(nome.rbegin(), nome.rend());
    cout << invertida << '\n';
}
