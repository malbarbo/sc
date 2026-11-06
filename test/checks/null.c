#include <stdio.h>
#include <stdlib.h>

struct node {
    int value;
    struct node *next;
};

int main(void) {
    struct node *list = NULL;
    printf("%d\n", list->value);
}
