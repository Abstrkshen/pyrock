# Multi-dimensional indexing and `@` on Python objects (numpy-style code).
import fractions


class Grid:
    def __getitem__(self, key):
        return repr(key)


class Mat:
    def __init__(self, v):
        self.v = v

    def __matmul__(self, other):
        return Mat(self.v * other.v)


g = Grid()
print(g[:, 4], g[1:3, ::2], g[0, :])
print((Mat(3) @ Mat(4)).v)
try:
    print(1 @ fractions.Fraction(1, 2))
except TypeError as e:
    print("TypeError:", e)
