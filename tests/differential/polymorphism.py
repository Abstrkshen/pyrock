class Shape:
    def __init__(self, name):
        self.name = name

    def area(self):
        return 0.0

    def __repr__(self):
        return f"<{self.name}>"


class Rect(Shape):
    def __init__(self, w, h):
        super().__init__("rect")
        self.w = w
        self.h = h

    def area(self):
        return self.w * self.h


class Circle(Shape):
    def __init__(self, r):
        super().__init__("circle")
        self.r = r

    def area(self):
        return 3.0 * self.r * self.r

    def grow(self, k):
        self.r *= k


def total_area(shapes):
    t = 0.0
    for s in shapes:
        t += s.area()
    return t


def largest(shapes):
    best = shapes[0]
    for s in shapes:
        if s.area() > best.area():
            best = s
    return best


shapes = [Rect(2.0, 3.0), Circle(1.0), Rect(1.0, 1.0)]
print(shapes, total_area(shapes), largest(shapes))
for s in shapes:
    print(s.name, s.area(), isinstance(s, Circle), isinstance(s, Shape))
    if isinstance(s, Circle):
        print("circle radius", s.area())
c = shapes[1]
print(c == shapes[1], shapes[0] == shapes[2], shapes.index(c))
by_name = {}
for s in shapes:
    by_name.setdefault(s.name, []).append(s)
print(by_name)
shapes.append(Circle(2.0))
print(len(shapes), round(total_area(shapes), 2))
