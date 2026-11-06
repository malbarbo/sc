// loop with a menu, switch and reading with scanf
#include <stdio.h>

int main(void) {
    int option;
    double balance = 0, amount;
    char line[50];
    while (fgets(line, sizeof line, stdin) != NULL && sscanf(line, "%d", &option) == 1) {
        switch (option) {
        case 1:
            amount = 10;
            balance += amount;
            break;
        case 2:
            if (balance >= 5) {
                balance -= 5;
            } else {
                printf("insufficient funds\n");
            }
            break;
        default:
            printf("invalid option\n");
            break;
        }
    }
    printf("balance: %.2f\n", balance);
    int age = 20;
    const char *group = age < 12 ? "child" : age < 18 ? "teenager" : "adult";
    printf("%s\n", group);
    return 0;
}
