#ifndef SIZES_H
#define SIZES_H
#define NB (sizeof(int) * 8)
typedef struct { int bits[(1024 + NB - 1) / NB]; } Fds;
int count_fds(Fds *f);
#endif
