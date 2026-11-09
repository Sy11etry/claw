#include <stdio.h>
#include <stdlib.h>

struct stack {
    int *items;
    int len;
    int cap;
};

void push(struct stack *s, int x) {
    if (s->len == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 4;
        s->items = realloc(s->items, s->cap * sizeof(int));
        if (!s->items) abort();
    }
    s->items[s->len++] = x;
}

int pop(struct stack *s, int *out) {
    if (s->len == 0) return 0;
    *out = s->items[--s->len];
    return 1;
}

int main(void) {
    struct stack s = {0};
    int v;
    push(&s, 1);
    push(&s, 2);
    printf("%d\n", pop(&s, &v) ? v : -1);
    printf("%d\n", pop(&s, &v) ? v : -1);
    printf("%d\n", pop(&s, &v) ? v : -1);
    free(s.items);
    return 0;
}
