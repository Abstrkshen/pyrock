#include "fastc.h"

int64_t gcd(int64_t a, int64_t b) {
    while (b != 0) {
        int64_t t = a % b;
        a = b;
        b = t;
    }
    return a < 0 ? -a : a;
}

double scale(double x, int k) { return x * k; }

const char *version(void) { return "fastc 1.0"; }
