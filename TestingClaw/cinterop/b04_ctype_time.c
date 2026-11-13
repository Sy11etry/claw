#include <stdio.h>
#include <ctype.h>
#include <time.h>

int main(void) {
    time_t t = time(NULL);
    printf("time %d\n", t > 1000000000 ? 1 : 0);
    printf("upper %d\n", toupper('q'));
    printf("digit %d\n", isdigit('7') != 0 ? 1 : 0);
    return 0;
}
