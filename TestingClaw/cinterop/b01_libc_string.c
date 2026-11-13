#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int sign(int r) { return r < 0 ? -1 : (r > 0 ? 1 : 0); }

int main(void) {
    printf("len %d\n", (int)strlen("hello"));
    printf("cmp %d\n", sign(strcmp("apple", "banana")));
    printf("same %d\n", sign(strcmp("same", "same")));
    char *p = strchr("claw", 'w');
    printf("chr %d\n", p ? (int)(unsigned char)p[0] : -1);
    char *q = strchr("claw", 'z');
    printf("missing %d\n", q ? (int)(unsigned char)q[0] : -1);
    char *buf = malloc(4);
    memcpy(buf, "xyz", 3);
    printf("byte %d\n", (int)(unsigned char)buf[1]);
    free(buf);
    return 0;
}
