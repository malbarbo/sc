#include <stdio.h>

int main(void) {
    unsigned balance = 10, withdrawal = 20;
    balance = balance - withdrawal;
    printf("balance: %u\n", balance);
}
