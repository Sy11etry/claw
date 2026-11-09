#include <stdio.h>
#include <stdlib.h>
int main(void) {
    int *v = malloc(8);
    free(v);
    printf("%d\n", v[0]);
    return 0;
}
