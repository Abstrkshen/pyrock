from dataclasses import dataclass, field


@dataclass
class Item:
    name: str
    price: float
    qty: int = 1
    tags: list = field(default_factory=list)

    def total(self):
        return self.price * self.qty


@dataclass
class Order:
    id: int
    items: list = field(default_factory=list)

    def __post_init__(self):
        self.created = "now"

    def add(self, item):
        self.items.append(item)
        return self

    def value(self):
        return sum(i.total() for i in self.items)


class Shape:
    sides = 0

    def __init__(self, name):
        self.name = name

    def describe(self):
        return f"{self.name} with {self.sides} sides, area {self.area():.2f}"

    def area(self):
        return 0.0


class Square(Shape):
    sides = 4

    def __init__(self, side):
        super().__init__("square")
        self.side = side

    def area(self):
        return self.side * self.side


class Circle(Shape):
    def __init__(self, r):
        super().__init__("circle")
        self.r = r

    def area(self):
        return 3.14159 * self.r * self.r


class AppError(Exception):
    pass


class NotFound(AppError):
    def __init__(self, key):
        super().__init__(f"missing key {key!r}")
        self.key = key


class Invalid(ValueError):
    def __init__(self, field_name, code):
        self.field_name = field_name
        self.code = code


def lookup(d, key):
    if key not in d:
        raise NotFound(key)
    return d[key]


i1 = Item("pen", 1.5, 4)
i2 = Item("book", 12.0, tags=["paper"])
print(i1, i2.tags, i1.total(), i1 == Item("pen", 1.5, 4), i1 == i2)
o = Order(7).add(i1).add(i2)
print(o.id, len(o.items), o.value(), o.created)
o2 = Order(8)
o2.items.append(Item("x", 1.0))
print(len(o.items), len(o2.items))

sq = Square(3.0)
ci = Circle(1.0)
print(sq.describe())
print(ci.describe())
print(isinstance(sq, Shape), isinstance(sq, Circle), isinstance(ci, (Square, Circle)), Square.sides, ci.sides)

data = {"a": 1}
try:
    lookup(data, "b")
except NotFound as e:
    print("NotFound:", e, e.key)
try:
    lookup(data, "c")
except AppError as e:
    print("AppError:", e)
try:
    raise Invalid("email", 42)
except ValueError as e:
    print("ValueError:", repr(str(e)))
try:
    raise Invalid("age", 7)
except Invalid as e:
    print("Invalid:", e.field_name, e.code)
try:
    raise AppError
except Exception as e:
    print("caught", type(e).__name__ if False else "AppError", str(e) == "")
print(lookup(data, "a"))
raise NotFound("zzz")
