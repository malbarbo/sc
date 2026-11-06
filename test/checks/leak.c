#include <stdio.h>
#include <stdlib.h>

struct node {
    int value;
    struct node *next;
};

struct node *insert(struct node *list, int value) {
    struct node *n = malloc(sizeof(struct node));
    n->value = value;
    n->next = list;
    return n;
}

int sum(int n) {
    struct node *list = NULL;
    for (int i = 1; i <= n; i++) {
        list = insert(list, i);
    }
    int s = 0;
    for (struct node *p = list; p != NULL; p = p->next) {
        s += p->value;
    }
    return s;
}

int main(void) {
    printf("%d\n", sum(3));
}
