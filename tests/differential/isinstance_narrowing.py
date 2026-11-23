# mode: native-only
# `isinstance` narrows a union of native classes (tests/known_bugs had the repros).


class Shape:
    def area(self):
        return 0.0


class Square(Shape):
    def __init__(self, side):
        self.side = side
        self.corners = [(0, 0), (side, 0), (side, side), (0, side)]

    def area(self):
        return float(self.side * self.side)


class Circle(Shape):
    def __init__(self, r):
        self.r = r

    def area(self):
        return 3.0 * self.r * self.r


class Label:
    def __init__(self, text, parent):
        self.text = text
        self.parent = parent


# One copy per class: in describe(Circle) the Square branch is decided false
# at compile time and isn't type-checked (`range(len(v.corners))` used to be
# rejected there).
def describe(v):
    if isinstance(v, Square):
        total = 0
        for i in range(len(v.corners)):
            total += v.corners[i][0]
        return "square " + str(v.side) + " " + str(total)
    return "other"


print(describe(Square(3)), describe(Circle(2)))


# A union: each branch sees one class.
def info(v):
    if isinstance(v, Square):
        return "side " + str(v.side) + ", corners " + str(len(v.corners))
    elif isinstance(v, Circle):
        return "radius " + str(v.r)
    else:
        return "label " + v.text


items = [Square(2), Circle(1), Label("x", None), Square(5)]
for it in items:
    print(info(it))


# `not isinstance`, early exits, `and` / `or`, tuples of classes.
def size(v):
    if not isinstance(v, (Square, Circle)):
        return -1
    if isinstance(v, Square) and v.side > 3:
        return 100
    if isinstance(v, Circle) or v.side == 0:
        return 1
    return v.side


print([size(it) for it in items])


# Conditional expressions and `while`.
def radius_or_side(v):
    return v.r if isinstance(v, Circle) else (v.side if isinstance(v, Square) else 0)


print([radius_or_side(it) for it in items])


def count_down(v):
    out = []
    while isinstance(v, Square) and v.side > 0:
        out.append(v.side)
        v = Square(v.side - 1)  # reassigned: narrowed again by the test
    return out


print(count_down(Square(4)), count_down(Circle(1)))


# A narrowed field passed where `T | None` is expected keeps the field's type.
def wrap(v):
    if isinstance(v, Label):
        return Label("child of " + v.text, v)
    return Label("orphan", None)


a = wrap(Label("root", None))
b = wrap(Circle(1))
for lab in [a, b]:
    print(lab.text, lab.parent is None)

# Methods on a narrowed value call that class's method directly.
total = 0.0
for it in items:
    if isinstance(it, (Square, Circle)):
        total += it.area()
print(total)
