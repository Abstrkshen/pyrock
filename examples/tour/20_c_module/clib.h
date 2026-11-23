#ifndef CLIB_H
#define CLIB_H
#include <stdbool.h>
#include <stdint.h>

int64_t gcd(int64_t a, int64_t b);
double dot3(double ax, double ay, double az, double bx, double by, double bz);
uint32_t checksum(const char *s);
bool is_even(int64_t n);
int add_int(int a, int b);
const char *version(void);

#endif
