#include <stdint.h>

/* 31-bit linear congruential generator (deterministic, portable). */
int64_t lcg(int64_t state) { return (state * 1103515245 + 12345) % 2147483648; }

double to_unit(int64_t state) { return (double)state / 2147483648.0; }
