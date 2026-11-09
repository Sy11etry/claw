#include <stdio.h>

void isort(int *a, int n) {
    for (int i = 1; i < n; i++) {
        int x = a[i];
        int j = i;
        while (j > 0 && a[j - 1] > x) {
            a[j] = a[j - 1];
            j--;
        }
        a[j] = x;
    }
}

int main(void) {
    int b[] = {5, 2, 9, 1, 7};
    isort(b, 5);
    printf("[");
    for (int i = 0; i < 5; i++) {
        printf(i ? ", %d" : "%d", b[i]);
    }
    puts("]");
    return 0;
}
