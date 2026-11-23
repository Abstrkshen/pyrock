# functools.cached_property: computed on first access, then stored per instance.
import functools
from functools import cached_property

calls = []


class Circle:
    def __init__(self, r):
        self.r = r

    @cached_property
    def area(self):
        calls.append(str(self.r))
        return 3.14159 * self.r * self.r

    @functools.cached_property
    def label(self):
        calls.append("label")
        return "circle " + str(self.r)


a = Circle(2.0)
b = Circle(1.0)
print(round(a.area, 3), round(a.area, 3), round(b.area, 3), a.label, a.label, calls)
