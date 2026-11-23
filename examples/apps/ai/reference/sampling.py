"""CPython reference for sampling.c (only used to check the demo's output)."""
import math


def softmax(x):
    m = max(x)
    for i in range(len(x)):
        x[i] = math.exp(x[i] - m)
    s = sum(x)
    for i in range(len(x)):
        x[i] /= s


def argmax(x):
    best = 0
    for i in range(1, len(x)):
        if x[i] > x[best]:
            best = i
    return best


def entropy(p):
    return -sum(v * math.log2(v) for v in p if v > 0)
