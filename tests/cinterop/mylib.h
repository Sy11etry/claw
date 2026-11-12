#ifndef MYLIB_H
#define MYLIB_H
#define MY_VERSION 3
#define MY_FLAG_A (1 << 0)
#define MY_FLAG_B (1 << 4)
#define MY_PI 3.5
enum my_color { MY_RED, MY_GREEN = 5, MY_BLUE };
typedef struct my_point { int x; int y; double w; } my_point;
typedef int (*my_cb)(int);
int my_add3(int a, int b, int c);
double my_scale(double v, float k);
void my_fill(my_point *p, int x, int y);
int my_apply(my_cb f, int v);
const char *my_name(void);
unsigned long long my_big(unsigned int a);
signed char my_neg8(void);
#endif
