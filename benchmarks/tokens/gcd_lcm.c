#include <stdio.h>

long gcd(long a, long b) {
    if (b == 0) return a;
    return gcd(b, a % b);
}

long lcm(long a, long b) {
    return a / gcd(a, b) * b;
}

int main(void) {
    printf("%ld\n", gcd(84, 36));
    printf("%ld\n", lcm(21, 6));
    return 0;
}
