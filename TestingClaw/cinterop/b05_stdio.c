#include <stdio.h>

int main(void) {
    char buf[64];
    int n = snprintf(buf, 64, "%d-%s", 42, "x");
    printf("len %d\n", n);
    printf("first %d\n", (int)(unsigned char)buf[0]);
    FILE *f = fopen("b05_out.txt", "w");
    fwrite(buf, 1, n, f);
    fclose(f);
    FILE *g = fopen("b05_out.txt", "r");
    char back[64];
    size_t got = fread(back, 1, 64, g);
    fclose(g);
    printf("read %d\n", (int)got);
    return 0;
}
