#include <stdio.h>
#include <stdlib.h>
#include <sqlite3.h>

static int on_row(void *arg, int n, char **vals, char **names) {
    (void)n; (void)names;
    long *acc = arg;
    acc[0] += 1;
    acc[1] += atol(vals[0]);
    return 0;
}

static long query(int which) {
    sqlite3 *db;
    if (sqlite3_open(":memory:", &db) != 0) return -1;
    sqlite3_exec(db, "CREATE TABLE t(x INTEGER); WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<1000) INSERT INTO t SELECT x FROM c;", NULL, NULL, NULL);
    long acc[2] = {0, 0};
    int rc = sqlite3_exec(db, "SELECT x FROM t", on_row, acc, NULL);
    sqlite3_close(db);
    if (which == 0) return acc[0];
    if (which == 1) return acc[1];
    return rc;
}

int main(void) {
    printf("rows %ld\n", query(0));
    printf("sum %ld\n", query(1));
    printf("exec %ld\n", query(2));
    return 0;
}
