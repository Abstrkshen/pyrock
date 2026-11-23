# One unannotated function, many concrete specializations.
# Try: pyrock explain examples/tour/06_generics.py
def add(a, b):
    return a + b


def twice(x):
    return add(x, x)


def clamp(v, lo, hi):
    if v < lo:
        return lo
    if v > hi:
        return hi
    return v


def mean3(a, b, c):
    return (a + b + c) / 3


print(twice(21), twice(1.25), twice("ab"))
print(add(1, 2.5), add(True, 1), add("py", "fast"))
print(clamp(15, 0, 10), clamp(-0.5, 0.0, 1.0), clamp("m", "a", "k"))
print(mean3(1, 2, 4), mean3(0.5, 1.5, 2.5))
