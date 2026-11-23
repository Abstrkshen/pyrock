# typing.NamedTuple classes: fields, defaults, repr, ==, ordering, hashing,
# len, indexing, iteration and unpacking, _asdict, _fields, methods.
from typing import NamedTuple


class Point(NamedTuple):
    x: int
    y: int = 0

    def norm1(self):
        return abs(self.x) + abs(self.y)


p = Point(3, -4)
q = Point(1)
print(p, q, p == Point(3, -4), p < q, sorted([p, q, Point(1, -1)]))
x, y = p
print(x, y, len(p), p[0], p[1], p[-1], list(p), p._asdict(), Point._fields, p.norm1())
print({p: "p"}[Point(3, -4)], max([p, q]))
try:
    p.x = 1
except AttributeError as e:
    print("AttributeError")
