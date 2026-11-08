#include <stdint.h>
#include <stdio.h>

static int64_t steps(int64_t start) {
    int64_t n = start;
    int64_t c = 0;
    while (n != 1) {
        if (n % 2 == 0) {
            n = n / 2;
        } else {
            n = 3 * n + 1;
        }
        c += 1;
    }
    return c;
}

int main(void) {
    int64_t best = 0;
    for (int64_t i = 1; i < 2000000; i++) {
        int64_t s = steps(i);
        if (s > best) best = s;
    }
    printf("%lld\n", (long long)best);
    return 0;
}
