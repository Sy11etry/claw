#include <stdio.h>
#include <iconv.h>

int main(void) {
    iconv_t cd = iconv_open("UTF-16LE", "UTF-8");
    char src[2] = {'h', 'i'};
    char dst[8];
    char *ip = src, *op = dst;
    size_t inleft = 2, outleft = 8;
    iconv(cd, &ip, &inleft, &op, &outleft);
    iconv_close(cd);
    printf("bytes %d\n", (int)(8 - outleft));
    printf("first %d\n", (int)(unsigned char)dst[0]);
    return 0;
}
