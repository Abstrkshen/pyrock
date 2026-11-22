import math
from math import sqrt, pi, floor


def checks():
    out = []
    for x in [0.0, 0.5, 1.0, 2.0, 10.0, 1e-300]:
        out.append(round(math.sin(x) + math.cos(x) * math.tan(x / 3), 12))
        out.append(round(math.exp(x / 4) + math.log(x + 1) + math.log10(x + 1) + math.log2(x + 1), 12))
        out.append(sqrt(x) + math.hypot(x, 3, 4))
    return out


print(checks())
print(math.floor(-2.5), math.ceil(2.1), math.trunc(-2.9), floor(7), math.floor(True))
print(pi, math.e, math.tau, math.inf, -math.inf, math.isnan(math.nan))
print(math.atan2(1, -1), math.degrees(pi), math.radians(180), math.fabs(-3), math.copysign(2, -0.0))
print(math.pow(2, 10), math.pow(2.5, 0.5), math.fmod(7.5, 2), math.fmod(-7, 3), math.remainder(7, 2), math.remainder(5, 2))
print(math.gcd(12, 18, 30), math.lcm(4, 6), math.factorial(10), math.comb(10, 3), math.perm(5, 2), math.isqrt(99))
print(math.isclose(0.1 + 0.2, 0.3), math.isclose(1, 1.1, rel_tol=0.2), math.isinf(1e308 * 10), math.isfinite(1.0))
print(math.log(8, 2), math.log1p(1e-10), math.expm1(1e-10), math.asinh(1), math.acosh(2), math.atanh(0.5), math.ldexp(1.5, 3))
total = 0.0
for i in range(1, 100000):
    total += math.sqrt(i) * math.sin(i * 0.001)
print(round(total, 6))
for bad in [lambda: math.sqrt(-1), lambda: math.log(0), lambda: math.exp(1000), lambda: math.floor(math.inf),
            lambda: math.floor(math.nan), lambda: math.factorial(-1), lambda: math.acos(2), lambda: math.isqrt(-4),
            lambda: math.pow(0, -1), lambda: math.fmod(1, 0)]:
    try:
        print(bad())
    except (ValueError, OverflowError) as e:
        print(type(e).__name__, e)
