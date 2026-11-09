#include <stdio.h>

enum kind { SQUARE, RECT, EMPTY };

struct shape {
    enum kind kind;
    long w;
    long h;
};

long area(struct shape x) {
    switch (x.kind) {
    case SQUARE:
        return x.w * x.w;
    case RECT:
        return x.w * x.h;
    case EMPTY:
        return 0;
    }
    return 0;
}

int main(void) {
    struct shape a[] = {{SQUARE, 3, 0}, {RECT, 2, 5}, {EMPTY, 0, 0}};
    long t = 0;
    for (int i = 0; i < 3; i++) {
        t += area(a[i]);
    }
    printf("%ld\n", t);
    return 0;
}
