int Lib_Scale(int x);
#define LIB_CLAMP(x) ((x) < 0 ? 0 : (x))
typedef union { int a; float b; } Either;
Either lib_make(int a, int b);
