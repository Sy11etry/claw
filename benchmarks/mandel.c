#include <stdint.h>
#include <stdio.h>

static int64_t iterations(double cx, double cy) {
    double x = 0.0, y = 0.0;
    int64_t n = 0;
    while (n < 200 && x * x + y * y <= 4.0) {
        double t = x * x - y * y + cx;
        y = 2.0 * x * y + cy;
        x = t;
        n += 1;
    }
    return n;
}

int main(void) {
    int64_t total = 0;
    for (int64_t py = 0; py < 1000; py++) {
        for (int64_t px = 0; px < 1000; px++) {
            total += iterations((double)px / 400.0 - 2.0, (double)py / 500.0 - 1.0);
        }
    }
    printf("%lld\n", (long long)total);
    return 0;
}
