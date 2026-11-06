#include <stdio.h>

int main(void) {
    int votes[3];
    int ballots[] = {0, 2, 1, 2, 2};
    for (int i = 0; i < 5; i++)
        votes[ballots[i]]++;
    for (int c = 0; c < 3; c++)
        if (votes[c] < 0 || votes[c] > 5)
            printf("candidate %d: impossible count\n", c);
        else
            printf("candidate %d: %d votes\n", c, votes[c]);
}
