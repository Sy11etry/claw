#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <zlib.h>

int main(void) {
    unsigned long n = 1048576;
    unsigned char *src = malloc(n);
    for (unsigned long i = 0; i < n; i++) src[i] = (unsigned char)(i % 7);
    unsigned long bound = compressBound(n);
    unsigned char *dst = malloc(bound);
    unsigned long dlen = bound;
    int rc = compress(dst, &dlen, src, n);
    unsigned char *back = malloc(n);
    unsigned long blen = n;
    int rc2 = uncompress(back, &blen, dst, dlen);
    int ok = rc == Z_OK && rc2 == Z_OK && blen == n && memcmp(src, back, n) == 0;
    printf("roundtrip %s\n", ok ? "ok" : "bad");
    printf("smaller %d\n", dlen < n ? 1 : 0);
    printf("crc %lu\n", crc32(0, src, (uInt)n));
    printf("adler %lu\n", adler32(1, src, (uInt)n));
    return 0;
}
