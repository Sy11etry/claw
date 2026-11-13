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
/* Structs by value, one of each shape the C rules treat differently. */
typedef struct { int a; int b; } my_ii;                 /* 8 bytes: one integer register */
typedef struct { long a; double b; } my_ld;            /* 16: integer + float (x86-64), two integer (arm64) */
typedef struct { double x; double y; } my_dd;          /* two doubles: float registers on both */
typedef struct { float r, g, b, a; } my_ffff;          /* four floats: four (arm64) or two (x86-64) float registers */
typedef struct { int a; short b; char c; } my_odd;     /* 8 bytes with padding */
typedef struct { char t[12]; } my_12;                  /* 12 bytes of chars */
typedef struct { long a, b, c; double d; } my_big4;    /* 32 bytes: returned through memory */
my_ii my_ii_make(int a, int b);
int my_ii_sum(my_ii v);
my_ld my_ld_twice(my_ld v);
my_dd my_dd_add(my_dd p, my_dd q);
my_ffff my_ffff_scale(my_ffff c, float k);
my_odd my_odd_bump(my_odd o);
my_12 my_12_upper(my_12 s);
my_big4 my_big4_make(long a, my_ii b, double d);
int my_mixed(int a, my_dd p, int b, my_ii q, double c);
#endif
