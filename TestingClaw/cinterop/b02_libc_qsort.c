#include <stdio.h>
#include <stdlib.h>

static int cmp_int(const void *a, const void *b) {
    int x = *(const int *)a, y = *(const int *)b;
    return x < y ? -1 : (x > y ? 1 : 0);
}

static int sort_at(int i) {
    int p[4] = {5, 2, 9, 1};
    qsort(p, 4, sizeof(int), cmp_int);
    return p[i];
}

int main(void) {
    int arr[4] = {1, 2, 5, 9};
    int k = 2, k7 = 7;
    int *r = bsearch(&k, arr, 4, sizeof(int), cmp_int);
    int *m = bsearch(&k7, arr, 4, sizeof(int), cmp_int);
    printf("sorted %d %d %d %d\n", sort_at(0), sort_at(1), sort_at(2), sort_at(3));
    printf("found %d\n", r ? *r : -1);
    printf("missing %d\n", m ? *m : -1);
    return 0;
}
