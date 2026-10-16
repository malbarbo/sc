#include <cstring>
#include <iostream>

class Nome {
    char *texto;
public:
    Nome(const char *s) : texto(new char[std::strlen(s) + 1]) { std::strcpy(texto, s); }
    ~Nome() { delete[] texto; }
    const char *c_str() const { return texto; }
};

int main() {
    Nome a("ana");
    Nome b = a;
    std::cout << b.c_str() << "\n";
}
