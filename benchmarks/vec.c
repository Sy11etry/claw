#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    int64_t *ptr;
    int64_t len, cap;
} Vec;

static void push(Vec *a, int64_t x) {
    if (a->len == a->cap) {
        int64_t cap = a->cap ? a->cap * 2 : 4;
        int64_t *p = malloc(cap * sizeof(int64_t));
        if (a->len) memmove(p, a->ptr, a->len * sizeof(int64_t));
        free(a->ptr);
        a->ptr = p;
        a->cap = cap;
    }
    a->ptr[a->len++] = x;
}

static int64_t round_once(int64_t n) {
    Vec a = {0};
    for (int64_t i = 0; i < n; i++) push(&a, i);
    int64_t sum = 0;
    for (int64_t i = 0; i < a.len; i++) sum += a.ptr[i];
    for (int64_t i = 0; i < n; i++) sum += a.ptr[i];
    free(a.ptr);
    return sum;
}

int main(void) {
    int64_t total = 0;
    for (int i = 0; i < 5; i++) total += round_once(4000000);
    printf("%lld\n", (long long)total);
    return 0;
}
