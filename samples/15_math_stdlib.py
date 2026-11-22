# Calling a C-extension module (math) from native code.
import math
from math import sqrt, pi


def hypot(a, b):
    return sqrt(a * a + b * b)


def circle(r):
    return pi * r * r


print(hypot(3.0, 4.0), round(circle(2.0), 4))
print(math.floor(-2.5), math.ceil(2.1), math.gcd(84, 36), math.factorial(20))
print(math.isclose(0.1 + 0.2, 0.3), math.inf > 1e308, math.log(math.e))
