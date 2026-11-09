#include <stdbool.h>
#include <stdio.h>

int main(void) {
    bool composite[100] = {false};
    int count = 0;
    for (int i = 2; i < 100; i++) {
        if (composite[i]) continue;
        count++;
        for (int j = i * i; j < 100; j += i) {
            composite[j] = true;
        }
    }
    printf("%d\n", count);
    return 0;
}
