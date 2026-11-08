#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

static bool is_prime(int64_t n) {
    if (n < 2) return false;
    for (int64_t d = 2; d * d <= n; d++) {
        if (n % d == 0) return false;
    }
    return true;
}

int main(void) {
    int64_t count = 0;
    for (int64_t n = 0; n < 3000000; n++) {
        if (is_prime(n)) count++;
    }
    printf("%lld\n", (long long)count);
    return 0;
}
