#ifndef SIZES_NOSIZEOF_H
#define SIZES_NOSIZEOF_H
#define NB 32
typedef struct { int bits[(1024 + NB - 1) / NB]; } Fds;
int count_fds(Fds *f);
#endif
