class Vec2:
    dims = 2
    created = 0

    def __init__(self, x, y=0.0):
        self.x = x
        self.y = y
        Vec2.created += 1

    def __add__(self, other):
        return Vec2(self.x + other.x, self.y + other.y)

    def __mul__(self, k):
        return Vec2(self.x * k, self.y * k)

    def __eq__(self, other):
        return self.x == other.x and self.y == other.y

    def __repr__(self):
        return f"Vec2({self.x}, {self.y})"

    def __len__(self):
        return 2

    def norm(self):
        return (self.x * self.x + self.y * self.y) ** 0.5

    def scale(self, k):
        self.x *= k
        self.y *= k
        return self

    @property
    def area(self):
        return self.x * self.y

    @staticmethod
    def zero():
        return Vec2(0.0, 0.0)

    @classmethod
    def unit(cls):
        return cls(1.0, 0.0)


class Counter:
    def __init__(self):
        self.counts = {}
        self.total = 0

    def add(self, word):
        self.counts[word] = self.counts.get(word, 0) + 1
        self.total += 1

    def most_common(self):
        best = None
        best_n = 0
        for w, n in self.counts.items():
            if n > best_n:
                best, best_n = w, n
        return best

    def __contains__(self, word):
        return word in self.counts

    def __getitem__(self, word):
        return self.counts.get(word, 0)


class Temperature:
    def __init__(self, c):
        self._c = c

    @property
    def fahrenheit(self):
        return self._c * 9 / 5 + 32

    @fahrenheit.setter
    def fahrenheit(self, f):
        self._c = (f - 32) * 5 / 9


a = Vec2(3.0, 4.0)
b = Vec2(1.0)
print(a, b, a + b, a * 2.0, a.norm(), a.area, len(a))
print(a == Vec2(3.0, 4.0), a != b, a is a, Vec2.dims, a.dims, Vec2.created)
alias = a
alias.scale(10.0)
print(a, alias is a, Vec2.zero(), Vec2.unit())
points = [Vec2(float(i), float(i * i)) for i in range(3)]
print(points, sum(p.x for p in points))

c = Counter()
for w in "the cat and the hat and the bat".split():
    c.add(w)
print(c.total, c.most_common(), "cat" in c, "dog" in c, c["the"], c["zzz"])

t = Temperature(100.0)
print(t.fahrenheit)
t.fahrenheit = 32.0
print(t._c, t.fahrenheit)
try:
    print(t.missing)
except AttributeError as e:
    print("AttributeError:", e)
