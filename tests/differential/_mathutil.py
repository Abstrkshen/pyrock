print("_mathutil imported once")


def square(x):
    return x * x


def clamp(v, lo, hi):
    if v < lo:
        return lo
    if v > hi:
        return hi
    return v


def describe(n):
    return "n=" + str(square(n))
