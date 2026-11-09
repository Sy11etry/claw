#include <stdio.h>

enum bank_err { OK, NO_FUNDS, NO_ACCOUNT };

enum bank_err withdraw(long *a, int len, int i, long n, long *short_by) {
    if (i < 0 || i >= len) return NO_ACCOUNT;
    if (a[i] < n) {
        *short_by = n - a[i];
        return NO_FUNDS;
    }
    a[i] -= n;
    return OK;
}

void attempt(long *a, int len, int i, long n) {
    long short_by = 0;
    switch (withdraw(a, len, i, n, &short_by)) {
    case OK:
        printf("ok %ld\n", a[i]);
        break;
    case NO_FUNDS:
        printf("short by %ld\n", short_by);
        break;
    case NO_ACCOUNT:
        puts("no account");
        break;
    }
}

int main(void) {
    long a[] = {100, 5};
    attempt(a, 2, 0, 30);
    attempt(a, 2, 1, 30);
    attempt(a, 2, 7, 1);
    return 0;
}
