import _mathutil
from _mathutil import clamp

print(_mathutil.square(7), _mathutil.square(1.5))
print(clamp(15, 0, 10), clamp(-3, 0, 10))
print(_mathutil.describe(3))
