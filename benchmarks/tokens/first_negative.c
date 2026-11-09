#include <stdbool.h>
#include <stdio.h>

bool first_neg(const int *a, int n, int *out) {
    for (int i = 0; i < n; i++) {
        if (a[i] < 0) {
            *out = a[i];
            return true;
        }
    }
    return false;
}

int main(void) {
    int a[] = {3, -4, 5};
    int b[] = {3, 4};
    int v;
    printf("%d\n", first_neg(a, 3, &v) ? v : 0);
    printf("%d\n", first_neg(b, 2, &v) ? v : 0);
    return 0;
}
