#include "clib.h"

int64_t gcd(int64_t a, int64_t b) {
    while (b != 0) {
        int64_t t = a % b;
        a = b;
        b = t;
    }
    return a < 0 ? -a : a;
}

double dot3(double ax, double ay, double az, double bx, double by, double bz) {
    return ax * bx + ay * by + az * bz;
}

/* FNV-1a hash of a NUL-terminated string. */
uint32_t checksum(const char *s) {
    uint32_t h = 2166136261u;
    while (*s) {
        h ^= (unsigned char)*s++;
        h *= 16777619u;
    }
    return h;
}

bool is_even(int64_t n) { return n % 2 == 0; }

int add_int(int a, int b) { return a + b; }

const char *version(void) { return "clib 0.1"; }
