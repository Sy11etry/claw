#include "mylib.h"
int my_add3(int a, int b, int c) { return a + b + c; }
double my_scale(double v, float k) { return v * k; }
void my_fill(my_point *p, int x, int y) { p->x = x; p->y = y; p->w = x * 0.5; }
int my_apply(my_cb f, int v) { return f(v) + f(-v); }
const char *my_name(void) { return "mylib"; }
unsigned long long my_big(unsigned int a) { return (unsigned long long)a * 4294967296ULL; }
signed char my_neg8(void) { return -5; }
