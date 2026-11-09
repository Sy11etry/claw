#include <stdio.h>

int search(const int *a, int n, int x) {
    int lo = 0;
    int hi = n;
    while (lo < hi) {
        int mid = lo + (hi - lo) / 2;
        if (a[mid] == x) return mid;
        if (a[mid] < x) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    return -1;
}

int main(void) {
    int a[] = {1, 3, 5, 7, 9, 11};
    printf("%d\n", search(a, 6, 7));
    printf("%d\n", search(a, 6, 4));
    return 0;
}
