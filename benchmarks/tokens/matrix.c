#include <stdio.h>

struct mat {
    long a[2][2];
};

struct mat mul(struct mat x, struct mat y) {
    struct mat r = {{{0}}};
    for (int i = 0; i < 2; i++) {
        for (int j = 0; j < 2; j++) {
            for (int k = 0; k < 2; k++) {
                r.a[i][j] += x.a[i][k] * y.a[k][j];
            }
        }
    }
    return r;
}

int main(void) {
    struct mat m = {{{1, 1}, {1, 0}}};
    struct mat p = m;
    for (int n = 0; n < 9; n++) {
        p = mul(p, m);
    }
    printf("%ld\n", p.a[0][1]);
    return 0;
}
