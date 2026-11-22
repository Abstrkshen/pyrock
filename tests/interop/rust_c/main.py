import vecmath
import fastc
from vecmath import checked_div


def norm2(x, y):
    return vecmath.dot(x, y, x, y)


print(norm2(3.0, 4.0), norm2(3, 4))
print(vecmath.greet("tests"))
print(fastc.gcd(48, 18), fastc.scale(1.5, 3), fastc.version())
print(checked_div(7, 2))
print(fastc.scale(2, 3000000000))
