#include <stdio.h>
#include <stdlib.h>

struct node {
    int v;
    struct node *left;
    struct node *right;
};

struct node *insert(struct node *t, int x) {
    if (!t) {
        t = calloc(1, sizeof *t);
        if (!t) abort();
        t->v = x;
        return t;
    }
    if (x < t->v) {
        t->left = insert(t->left, x);
    } else {
        t->right = insert(t->right, x);
    }
    return t;
}

void walk(const struct node *t, int *first) {
    if (!t) return;
    walk(t->left, first);
    printf(*first ? "%d" : ", %d", t->v);
    *first = 0;
    walk(t->right, first);
}

void release(struct node *t) {
    if (!t) return;
    release(t->left);
    release(t->right);
    free(t);
}

int main(void) {
    int xs[] = {5, 2, 8, 1, 9};
    struct node *t = NULL;
    for (int i = 0; i < 5; i++) {
        t = insert(t, xs[i]);
    }
    int first = 1;
    printf("[");
    walk(t, &first);
    puts("]");
    release(t);
    return 0;
}
