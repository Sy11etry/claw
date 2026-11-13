#ifndef MYLIB_H
#define MYLIB_H
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define MYLIB_MAX 42
#define MYLIB_MASK 0xFF
#define MYLIB_TWICE (MYLIB_MAX * 2)
#define MYLIB_RATIO 3.25

typedef enum { RED, GREEN = 5, BLUE } Color;

typedef struct {
    int32_t x;
    int32_t y;
} Point;

typedef int (*binop)(int, int);

int mylib_add(int a, int b);
int32_t mylib_scale(const Point *p, int32_t k);
void mylib_move(Point *p, int32_t dx, int32_t dy);
int mylib_apply(binop f, int a, int b);
uint64_t mylib_big(uint8_t small, uint64_t big);
size_t mylib_len(const char *s);
bool mylib_is_even(int n);
const char *mylib_name(void);
int mylib_pair(int *a, int *b);
Color mylib_next(Color c);

#endif
