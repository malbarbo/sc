#include <cstring>
#include <iostream>

class Name {
    char *text;
public:
    Name(const char *s) : text(new char[std::strlen(s) + 1]) { std::strcpy(text, s); }
    ~Name() { delete[] text; }
    const char *c_str() const { return text; }
};

int main() {
    Name a("ana");
    Name b = a;
    std::cout << b.c_str() << "\n";
}
