// C arrays (pointer + length) and structs by value.
// pyfast: link m
#include <math.h>
#include <stddef.h>

typedef struct {
    double x, y;
} Vec2;

typedef struct {
    unsigned char r;
    unsigned char g;
    unsigned char b;
} Color;

double dot(const double *a, size_t n) {
    double s = 0;
    for (size_t i = 0; i < n; i++) s += a[i] * a[i];
    return sqrt(s);
}

void normalize(double *data, int n) {
    double m = 0;
    for (int i = 0; i < n; i++) if (data[i] > m) m = data[i];
    for (int i = 0; i < n; i++) data[i] /= m;
}

Vec2 vadd(Vec2 a, Vec2 b) {
    Vec2 r = {a.x + b.x, a.y + b.y};
    return r;
}

int brightness(Color c) { return (c.r + c.g + c.b) / 3; }

double integrate(double (*f)(double), double a, double b, int n) {
    double h = (b - a) / n, s = 0;
    for (int i = 0; i < n; i++) s += f(a + (i + 0.5) * h) * h;
    return s;
}
