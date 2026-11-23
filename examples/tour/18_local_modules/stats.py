import geometry       # already imported by main: its body does not run again


def mean(a, b, c):
    return (a + b + c) / 3


def variance(a, b, c):
    m = mean(a, b, c)
    return ((a - m) ** 2 + (b - m) ** 2 + (c - m) ** 2) / 3
