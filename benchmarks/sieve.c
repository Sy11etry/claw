#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#define SIZE 5000000

static int64_t count(void) {
    bool composite[SIZE];
    for (int64_t i = 0; i < SIZE; i++) composite[i] = false;
    int64_t found = 0;
    for (int64_t i = 2; i < SIZE; i++) {
        if (composite[i]) continue;
        found += 1;
        for (int64_t j = i * i; j < SIZE; j += i) composite[j] = true;
    }
    return found;
}

int main(void) {
    int64_t total = 0;
    for (int i = 0; i < 10; i++) total += count();
    printf("%lld\n", (long long)total);
    return 0;
}
