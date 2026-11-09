#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct entry {
    char word[32];
    int count;
};

static int by_word(const void *a, const void *b) {
    return strcmp(((const struct entry *)a)->word, ((const struct entry *)b)->word);
}

int main(void) {
    char text[] = "the cat and the dog and the bird";
    struct entry items[16];
    int n = 0;
    for (char *w = strtok(text, " "); w; w = strtok(NULL, " ")) {
        int i = 0;
        while (i < n && strcmp(items[i].word, w) != 0) i++;
        if (i == n) {
            strcpy(items[n].word, w);
            items[n].count = 0;
            n++;
        }
        items[i].count++;
    }
    qsort(items, n, sizeof items[0], by_word);
    for (int i = 0; i < n; i++) {
        printf("%s %d\n", items[i].word, items[i].count);
    }
    return 0;
}
