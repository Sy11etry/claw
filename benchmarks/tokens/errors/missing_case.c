#include <stdio.h>
enum c { R, G, B };
int f(enum c x) {
    switch (x) {
    case R:
        return 1;
    case G:
        return 2;
    }
    return 0;
}
int main(void) {
    printf("%d\n", f(B));
    return 0;
}
