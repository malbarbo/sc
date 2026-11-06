// linked list: sorted insert, remove, print and free
#include <stdio.h>
#include <stdlib.h>

typedef struct Node {
    int value;
    struct Node *next;
} Node;

Node *insert(Node *list, int value) {
    Node *node = malloc(sizeof(Node));
    if (node == NULL) {
        exit(1);
    }
    node->value = value;
    if (list == NULL || value < list->value) {
        node->next = list;
        return node;
    }
    Node *p = list;
    while (p->next != NULL && p->next->value < value) {
        p = p->next;
    }
    node->next = p->next;
    p->next = node;
    return list;
}

Node *remove_value(Node *list, int value) {
    Node *prev = NULL;
    for (Node *p = list; p != NULL; prev = p, p = p->next) {
        if (p->value == value) {
            if (prev == NULL) {
                list = p->next;
            } else {
                prev->next = p->next;
            }
            free(p);
            break;
        }
    }
    return list;
}

void print(const Node *list) {
    printf("[");
    for (const Node *p = list; p != NULL; p = p->next) {
        printf("%d%s", p->value, p->next ? ", " : "");
    }
    printf("]\n");
}

int length(const Node *list) {
    return list == NULL ? 0 : 1 + length(list->next);
}

void free_list(Node *list) {
    while (list != NULL) {
        Node *next = list->next;
        free(list);
        list = next;
    }
}

int main(void) {
    Node *list = NULL;
    int values[] = {5, 2, 8, 1, 9, 3};
    int n = sizeof(values) / sizeof(values[0]);
    for (int i = 0; i < n; i++) {
        list = insert(list, values[i]);
    }
    print(list);
    list = remove_value(list, 8);
    list = remove_value(list, 1);
    print(list);
    printf("length: %d\n", length(list));
    free_list(list);
    return 0;
}
