#include <stdio.h>

struct point {
    long x;
    long y;
};

struct point add(struct point a, struct point b) {
    return (struct point){a.x + b.x, a.y + b.y};
}

long dist2(struct point a, struct point b) {
    long dx = a.x - b.x;
    long dy = a.y - b.y;
    return dx * dx + dy * dy;
}

int main(void) {
    struct point p = {1, 2};
    struct point q = {4, 6};
    struct point r = add(p, q);
    printf("%ld %ld\n", r.x, r.y);
    printf("%ld\n", dist2(p, q));
    return 0;
}
