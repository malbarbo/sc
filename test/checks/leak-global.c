#include <stdio.h>
#include <stdlib.h>

struct node {
    int value;
    struct node *next;
};

struct node *list = NULL;

int main(void) {
    for (int i = 1; i <= 3; i++) {
        struct node *n = malloc(sizeof(struct node));
        n->value = i;
        n->next = list;
        list = n;
    }
    printf("%d\n", list->value);
}
