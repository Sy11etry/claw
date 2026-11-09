#include <stdio.h>

#define MAX(x, y) ((x) > (y) ? (x) : (y))

struct pair {
    int a;
    const char *b;
};

int main(void) {
    printf("%d\n", MAX(3, 9));
    printf("%g\n", MAX(2.5, 1.5));
    struct pair p = {1, "one"};
    printf("%d %s\n", p.a, p.b);
    return 0;
}
