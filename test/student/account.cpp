// class with an invariant, exceptions and inheritance
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
using namespace std;

class Account {
    string owner;
    double balance = 0;

public:
    explicit Account(string owner) : owner(std::move(owner)) {}
    virtual ~Account() = default;
    void deposit(double amount) {
        if (amount <= 0) throw invalid_argument("invalid amount");
        balance += amount;
    }
    virtual void withdraw(double amount) {
        if (amount > balance) throw runtime_error("insufficient funds");
        balance -= amount;
    }
    double getBalance() const { return balance; }
    const string &getOwner() const { return owner; }
};

class Special : public Account {
    double limit;

public:
    Special(string owner, double limit) : Account(std::move(owner)), limit(limit) {}
    void withdraw(double amount) override {
        if (amount > getBalance() + limit) throw runtime_error("limit exceeded");
        deposit(limit);
        Account::withdraw(amount);
    }
};

int main() {
    vector<unique_ptr<Account>> accounts;
    accounts.push_back(make_unique<Account>("Ana"));
    accounts.push_back(make_unique<Special>("Bruno", 100));
    for (auto &c : accounts) {
        c->deposit(50);
        try {
            c->withdraw(120);
        } catch (const exception &e) {
            cout << c->getOwner() << ": " << e.what() << '\n';
        }
        cout << c->getOwner() << " has " << c->getBalance() << '\n';
    }
}
