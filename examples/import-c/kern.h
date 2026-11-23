#ifndef KERN_H
#define KERN_H
#include <stdint.h>

double lerp(double a, double b, double t);
int64_t step_pos(int64_t pos, int64_t vel, int64_t lo, int64_t hi);
int64_t step_vel(int64_t pos, int64_t vel, int64_t lo, int64_t hi);
const char *language(void);

#endif
