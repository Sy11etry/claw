#include <stdio.h>

int main(void) {
    long a[20] = {0, 1};
    for (int i = 2; i < 20; i++) {
        a[i] = a[i - 1] + a[i - 2];
    }
    long sum = 0;
    for (int i = 0; i < 20; i++) {
        sum += a[i];
    }
    printf("%ld %ld\n", a[19], sum);
    return 0;
}
