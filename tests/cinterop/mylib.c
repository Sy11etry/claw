#include "mylib.h"
int my_add3(int a, int b, int c) { return a + b + c; }
double my_scale(double v, float k) { return v * k; }
void my_fill(my_point *p, int x, int y) { p->x = x; p->y = y; p->w = x * 0.5; }
int my_apply(my_cb f, int v) { return f(v) + f(-v); }
const char *my_name(void) { return "mylib"; }
unsigned long long my_big(unsigned int a) { return (unsigned long long)a * 4294967296ULL; }
signed char my_neg8(void) { return -5; }
my_ii my_ii_make(int a, int b) { my_ii r = { a, b }; return r; }
int my_ii_sum(my_ii v) { return v.a + v.b; }
my_ld my_ld_twice(my_ld v) { my_ld r = { v.a * 2, v.b * 2 }; return r; }
my_dd my_dd_add(my_dd p, my_dd q) { my_dd r = { p.x + q.x, p.y + q.y }; return r; }
my_ffff my_ffff_scale(my_ffff c, float k) { my_ffff r = { c.r * k, c.g * k, c.b * k, c.a * k }; return r; }
my_odd my_odd_bump(my_odd o) { my_odd r = { o.a + 1, o.b + 1, o.c + 1 }; return r; }
my_12 my_12_upper(my_12 s) { for (int i = 0; i < 12; i++) if (s.t[i] >= 'a' && s.t[i] <= 'z') s.t[i] -= 32; return s; }
my_big4 my_big4_make(long a, my_ii b, double d) { my_big4 r = { a, b.a, b.b, d }; return r; }
int my_mixed(int a, my_dd p, int b, my_ii q, double c) { return a + (int)p.x + (int)p.y + b + q.a + q.b + (int)c; }
