#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct Node {
    struct Node *next;
    int64_t v;
} Node;

static int64_t round_once(int64_t n) {
    Node *head = NULL;
    for (int64_t i = 0; i < n; i++) {
        Node *p = malloc(sizeof(Node));
        p->next = head;
        p->v = i;
        head = p;
    }
    int64_t sum = 0;
    while (head != NULL) {
        sum += head->v;
        Node *next = head->next;
        free(head);
        head = next;
    }
    return sum;
}

int main(void) {
    int64_t total = 0;
    for (int i = 0; i < 5; i++) total += round_once(1000000);
    printf("%lld\n", (long long)total);
    return 0;
}
