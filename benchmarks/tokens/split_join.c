#include <ctype.h>
#include <stdio.h>
#include <string.h>

int main(void) {
    char text[] = "a,b,c";
    char out[16] = "";
    int n = 0;
    for (char *p = strtok(text, ","); p; p = strtok(NULL, ",")) {
        if (n++) strcat(out, "-");
        strcat(out, p);
    }
    printf("%d\n%s\n", n, out);
    char s[] = "  claw ";
    char *b = s;
    while (isspace((unsigned char)*b)) b++;
    char *e = b + strlen(b);
    while (e > b && isspace((unsigned char)e[-1])) e--;
    *e = 0;
    for (char *p = b; *p; p++) {
        *p = toupper((unsigned char)*p);
    }
    puts(b);
    return 0;
}
