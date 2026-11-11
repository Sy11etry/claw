#include <stdio.h>
#include <stdlib.h>

int main(void) {
    int n = 1024;
    double *a = malloc((size_t)n * n * 8), *b = malloc((size_t)n * n * 8), *c = calloc((size_t)n * n, 8);
    for (int i = 0; i < n * n; i++) {
        a[i] = (i % 7) + 1;
        b[i] = (i % 5) + 1;
    }
    for (int i = 0; i < n; i++) {
        for (int k = 0; k < n; k++) {
            double x = a[i * n + k];
            for (int j = 0; j < n; j++) c[i * n + j] += x * b[k * n + j];
        }
    }
    double sum = 0;
    for (int i = 0; i < n * n; i++) sum += c[i];
    printf("%lld\n", (long long)sum);
    free(a); free(b); free(c);
    return 0;
}
