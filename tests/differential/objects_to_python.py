import json


class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y
        self.tags = []

    def norm2(self):
        return self.x * self.x + self.y * self.y

    def scaled(self, k=2):
        return Point(self.x * k, self.y * k)

    def __repr__(self):
        return f"Point({self.x}, {self.y})"


def to_dict(p):
    return {"x": p.x, "y": p.y}


pts = [Point(3, 4), Point(1, 1), Point(0, 2)]
# Python's sorted/max/getattr work on native instances.
by_norm = sorted(pts, key=lambda p: p.norm2())
print(by_norm)
print(max(pts, key=lambda p: p.x))
print(json.dumps([to_dict(p) for p in pts]))
print(json.dumps(pts, default=to_dict))
getter = getattr(pts[0], "norm2")
print(getter(), getattr(pts[0], "x"), repr(pts[1]))
scale = getattr(pts[2], "scaled")
print(scale(), scale(k=5))
setattr(pts[1], "x", 10)
print(pts[1], pts[1].x + 1)
p = pts[0]
p.tags.append("first")
copy = [q for q in pts if q is p]
print(copy[0] is p, p.tags)
