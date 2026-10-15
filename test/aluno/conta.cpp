// classe com invariante, exceções e herança
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
using namespace std;

class Conta {
    string titular;
    double saldo = 0;

public:
    explicit Conta(string titular) : titular(std::move(titular)) {}
    virtual ~Conta() = default;
    void deposita(double valor) {
        if (valor <= 0) throw invalid_argument("valor invalido");
        saldo += valor;
    }
    virtual void saca(double valor) {
        if (valor > saldo) throw runtime_error("saldo insuficiente");
        saldo -= valor;
    }
    double getSaldo() const { return saldo; }
    const string &getTitular() const { return titular; }
};

class Especial : public Conta {
    double limite;

public:
    Especial(string titular, double limite) : Conta(std::move(titular)), limite(limite) {}
    void saca(double valor) override {
        if (valor > getSaldo() + limite) throw runtime_error("limite excedido");
        deposita(limite);
        Conta::saca(valor);
    }
};

int main() {
    vector<unique_ptr<Conta>> contas;
    contas.push_back(make_unique<Conta>("Ana"));
    contas.push_back(make_unique<Especial>("Bruno", 100));
    for (auto &c : contas) {
        c->deposita(50);
        try {
            c->saca(120);
        } catch (const exception &e) {
            cout << c->getTitular() << ": " << e.what() << '\n';
        }
        cout << c->getTitular() << " tem " << c->getSaldo() << '\n';
    }
}
