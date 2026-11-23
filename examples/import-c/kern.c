#include "kern.h"

double lerp(double a, double b, double t) {
    return a + (b - a) * t;
}

static int hits_wall(int64_t pos, int64_t vel, int64_t lo, int64_t hi) {
    int64_t next = pos + vel;
    return next < lo || next > hi;
}

int64_t step_vel(int64_t pos, int64_t vel, int64_t lo, int64_t hi) {
    if (hits_wall(pos, vel, lo, hi)) {
        return -vel;
    }
    return vel;
}

int64_t step_pos(int64_t pos, int64_t vel, int64_t lo, int64_t hi) {
    return pos + vel;
}

const char *language(void) {
    return "c";
}
