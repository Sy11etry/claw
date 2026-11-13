/* An array size from a macro that uses sizeof, as in the macOS fd_set. */
#define WORD_BITS (sizeof(int) * 8)
typedef struct { int bits[(256 + WORD_BITS - 1) / WORD_BITS]; long tail; } Bitset;
typedef struct { char tag; double value; } Pair;
#define PAIR_SIZE sizeof(Pair)
int bitset_words(const Bitset *b);
