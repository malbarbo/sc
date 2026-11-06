#include <stdio.h>

int main(void) {
    char name[8];
    int age;
    printf("Name and age: ");
    if (scanf("%s %d", name, &age) == 2) {
        printf("%s is %d years old\n", name, age);
    }
}
