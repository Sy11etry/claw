#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <bzlib.h>

int main(void) {
    unsigned int n = 100000;
    unsigned char *src = malloc(n);
    for (unsigned int i = 0; i < n; i++) src[i] = (unsigned char)(i % 11);
    unsigned int cap = n + n / 10 + 1024;
    char *dst = malloc(cap);
    unsigned int dlen = cap;
    int rc = BZ2_bzBuffToBuffCompress(dst, &dlen, (char *)src, n, 9, 0, 30);
    char *back = malloc(n);
    unsigned int blen = n;
    int rc2 = BZ2_bzBuffToBuffDecompress(back, &blen, dst, dlen, 0, 0);
    int ok = rc == BZ_OK && rc2 == BZ_OK && blen == n && memcmp(src, back, n) == 0;
    printf("ok %d\n", ok ? 1 : 0);
    printf("smaller %d\n", dlen < n ? 1 : 0);
    return 0;
}
