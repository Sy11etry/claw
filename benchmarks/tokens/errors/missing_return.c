#include <stdio.h>
int f(int a) {
    if (a > 0) {
        return 1;
    }
}
int main(void) {
    printf("%d\n", f(1));
    return 0;
}
