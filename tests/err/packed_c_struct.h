/* Structs that C lays out differently from Claw: naming one is an error (E0182), not a silent layout. */
struct __attribute__((packed)) Pk { char c; int i; };
typedef struct { char c; int i; } __attribute__((packed)) TPk;
#pragma pack(1)
struct Pp { char c; int i; };
#pragma pack()
struct __attribute__((aligned(16))) Al { int x; };
int pk_size(struct Pk *p);
