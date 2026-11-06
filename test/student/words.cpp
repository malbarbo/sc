// string and map: counting the words read from the input
#include <cctype>
#include <iostream>
#include <map>
#include <sstream>
#include <string>
using namespace std;

string to_lower(string s) {
    for (char &c : s) c = tolower(c);
    return s;
}

int main() {
    map<string, int> counts;
    string line;
    int lines = 0;
    while (getline(cin, line)) {
        lines++;
        istringstream in(line);
        string word;
        while (in >> word) counts[to_lower(word)]++;
    }
    cout << lines << " lines\n";
    for (const auto &[word, n] : counts) cout << word << ": " << n << '\n';
    string name = "Maria da Silva";
    auto pos = name.find(' ');
    cout << name.substr(0, pos) << " / " << name.size() << '\n';
    string reversed(name.rbegin(), name.rend());
    cout << reversed << '\n';
}
