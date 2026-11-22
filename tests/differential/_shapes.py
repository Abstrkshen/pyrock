class Square:
    def __init__(self, side):
        self.side = side

    def area(self):
        return self.side * self.side


def area_sum(shapes):
    return sum(s.area() for s in shapes)
