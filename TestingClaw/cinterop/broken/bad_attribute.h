#ifndef BAD_ATTR_H
#define BAD_ATTR_H
struct __attribute__((packed)) Pk { char c; int i; };
int pk_size(struct Pk *p);
#endif
