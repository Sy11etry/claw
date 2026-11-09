#include <stdbool.h>
#include <stdio.h>
#include <string.h>

bool is_pal(const char *s) {
    size_t n = strlen(s);
    for (size_t i = 0; i < n / 2; i++) {
        if (s[i] != s[n - 1 - i]) return false;
    }
    return true;
}

int main(void) {
    puts(is_pal("level") ? "true" : "false");
    puts(is_pal("claw") ? "true" : "false");
    puts(is_pal("") ? "true" : "false");
    return 0;
}
