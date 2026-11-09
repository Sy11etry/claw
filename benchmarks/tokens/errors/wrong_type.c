#include <stdio.h>
#include <string.h>
int f(const char *s) {
    return strlen(s);
}
int main(void) {
    printf("%d\n", f(5));
    return 0;
}
