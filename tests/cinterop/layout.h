struct Plain { char c; int i; };
int plain_size(struct Plain *p);
struct __attribute__((packed)) Pk { char c; int i; };
int pk_size(struct Pk *p);
