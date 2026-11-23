def name():
    return "shapes"


def circle_area(r):
    return 3.141592653589793 * r * r


def ring_area(outer, inner):
    return circle_area(outer) - circle_area(inner)


def diameter(r):
    return r + r
