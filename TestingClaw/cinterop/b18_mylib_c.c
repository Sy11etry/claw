#include <stdio.h>
#include "mylib.h"

static int mul_cb(int a, int b) { return a * b; }

int main(void) {
    printf("add %d\n", mylib_add(2, 3));
    Point p = {1, 2};
    printf("scale %d\n", mylib_scale(&p, 3));
    mylib_move(&p, 10, 20);
    printf("moved %d\n", mylib_scale(&p, 3));
    printf("apply %d\n", mylib_apply(mul_cb, 6, 7));
    printf("big %llu\n", (unsigned long long)mylib_big(200, 1000000000000ULL));
    printf("len %d\n", (int)mylib_len("hello"));
    printf("even %s\n", mylib_is_even(4) ? "true" : "false");
    printf("name %d\n", mylib_name()[0]);
    int a = 0, b = 0;
    int rc = mylib_pair(&a, &b);
    printf("pair %d\n", rc * 100 + a * 10 + b);
    printf("next %d\n", mylib_next(RED));
    printf("consts %d %d %d %d\n", MYLIB_MAX, MYLIB_MASK, MYLIB_TWICE, (int)(MYLIB_RATIO * 4.0));
    printf("blue %d\n", BLUE);
    return 0;
}
