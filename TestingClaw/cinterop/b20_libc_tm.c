#include <stdio.h>
#include <time.h>
int main(void) {
    time_t t = 0;
    struct tm *tmp = gmtime(&t);
    printf("year %d\n", tmp->tm_year);
    printf("mon %d\n", tmp->tm_mon);
    printf("mday %d\n", tmp->tm_mday);
    printf("hour %d\n", tmp->tm_hour);
    return 0;
}
