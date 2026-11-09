#include <stdio.h>

int average(const long *xs, int n, long *out) {
    if (n == 0) return -1;
    long sum = 0;
    for (int i = 0; i < n; i++) {
        sum += xs[i];
    }
    *out = sum / n;
    return 0;
}

int main(void) {
    long xs[] = {2, 4, 9};
    long v;
    printf("%ld\n", average(xs, 3, &v) == 0 ? v : -1);
    printf("%ld\n", average(xs, 0, &v) == 0 ? v : -1);
    return 0;
}
