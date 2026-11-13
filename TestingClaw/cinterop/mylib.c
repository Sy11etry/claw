#include "mylib.h"
#include <string.h>

int mylib_add(int a, int b) { return a + b; }
int32_t mylib_scale(const Point *p, int32_t k) { return (p->x + p->y) * k; }
void mylib_move(Point *p, int32_t dx, int32_t dy) { p->x += dx; p->y += dy; }
int mylib_apply(binop f, int a, int b) { return f(a, b); }
uint64_t mylib_big(uint8_t small, uint64_t big) { return big * 2 + small; }
size_t mylib_len(const char *s) { return strlen(s); }
bool mylib_is_even(int n) { return n % 2 == 0; }
const char *mylib_name(void) { return "mylib"; }
int mylib_pair(int *a, int *b) { *a = 7; *b = 9; return 2; }
Color mylib_next(Color c) { return c == RED ? GREEN : (c == GREEN ? BLUE : RED); }
