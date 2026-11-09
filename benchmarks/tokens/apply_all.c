#include <stdio.h>

int twice(int x) {
    return x * 2;
}

int square(int x) {
    return x * x;
}

void apply(const int *a, int n, int (*f)(int)) {
    printf("[");
    for (int i = 0; i < n; i++) {
        printf(i ? ", %d" : "%d", f(a[i]));
    }
    puts("]");
}

int main(void) {
    int a[] = {1, 2, 3};
    apply(a, 3, twice);
    apply(a, 3, square);
    return 0;
}
