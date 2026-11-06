#include <stdio.h>
#include <stdlib.h>

struct node {
    int value;
    struct node *next;
};

void free_list(struct node *list) {
    while (list != NULL) {
        free(list);
        list = list->next;
    }
}

int main(void) {
    struct node *list = NULL;
    for (int i = 0; i < 3; i++) {
        struct node *n = malloc(sizeof(struct node));
        n->value = i;
        n->next = list;
        list = n;
    }
    free_list(list);
    printf("ok\n");
}
