// structs: records, average, sorting with qsort and search
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    char name[30];
    double grades[3];
    double average;
} Student;

double average(const double grades[], int n) {
    double sum = 0;
    for (int i = 0; i < n; i++) {
        sum += grades[i];
    }
    return sum / n;
}

int compare(const void *a, const void *b) {
    const Student *x = a;
    const Student *y = b;
    if (x->average < y->average) return 1;
    if (x->average > y->average) return -1;
    return strcmp(x->name, y->name);
}

int find(const Student v[], int n, const char *name) {
    for (int i = 0; i < n; i++) {
        if (strcmp(v[i].name, name) == 0) {
            return i;
        }
    }
    return -1;
}

int main(void) {
    Student students[] = {
        {"Bruno", {7.5, 8.0, 6.5}, 0},
        {"Ana", {9.0, 9.5, 10.0}, 0},
        {"Carla", {5.0, 6.0, 7.0}, 0},
    };
    int n = sizeof students / sizeof students[0];
    for (int i = 0; i < n; i++) {
        students[i].average = average(students[i].grades, 3);
    }
    qsort(students, n, sizeof(Student), compare);
    for (int i = 0; i < n; i++) {
        printf("%-10s %5.2f %s\n", students[i].name, students[i].average, students[i].average >= 7 ? "passed" : "exam");
    }
    printf("Carla at position %d\n", find(students, n, "Carla"));
    return 0;
}
