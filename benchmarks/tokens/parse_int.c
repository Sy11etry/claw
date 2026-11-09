#include <stdio.h>

enum parse_err { OK, EMPTY, BAD };

enum parse_err parse(const char *s, long *out, int *at) {
    if (!*s) return EMPTY;
    long n = 0;
    for (int i = 0; s[i]; i++) {
        if (s[i] < '0' || s[i] > '9') {
            *at = i;
            return BAD;
        }
        n = n * 10 + (s[i] - '0');
    }
    *out = n;
    return OK;
}

void show(const char *s) {
    long v;
    int at;
    switch (parse(s, &v, &at)) {
    case OK:
        printf("%ld\n", v);
        break;
    case EMPTY:
        puts("empty");
        break;
    case BAD:
        printf("bad at %d\n", at);
        break;
    }
}

int main(void) {
    show("123");
    show("");
    show("12x");
    return 0;
}
