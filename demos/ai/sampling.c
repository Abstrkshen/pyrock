// Sampling helpers over arrays (pointer + length).
// pyrock: link m
#include <math.h>
#include <stddef.h>

void softmax(double *x, int n) {
    double m = x[0], s = 0;
    for (int i = 1; i < n; i++) if (x[i] > m) m = x[i];
    for (int i = 0; i < n; i++) { x[i] = exp(x[i] - m); s += x[i]; }
    for (int i = 0; i < n; i++) x[i] /= s;
}

int argmax(const double *x, size_t n) {
    size_t best = 0;
    for (size_t i = 1; i < n; i++) if (x[i] > x[best]) best = i;
    return (int)best;
}

double entropy(const double *p, size_t n) {
    double h = 0;
    for (size_t i = 0; i < n; i++) if (p[i] > 0) h -= p[i] * log2(p[i]);
    return h;
}
