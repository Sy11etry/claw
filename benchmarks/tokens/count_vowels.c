#include <stdio.h>
#include <string.h>

int vowels(const char *s) {
    int n = 0;
    for (; *s; s++) {
        if (strchr("aeiou", *s)) n++;
    }
    return n;
}

int main(void) {
    printf("%d\n", vowels("the quick brown fox"));
    return 0;
}
