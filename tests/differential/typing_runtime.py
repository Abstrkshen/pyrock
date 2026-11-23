# Typing constructs that only matter to type checkers: Protocol and Generic
# bases, TypeVar, NewType, TypedDict.
from typing import Generic, NewType, Protocol, TypedDict, TypeVar

T = TypeVar("T")
UserId = NewType("UserId", int)


class Shape(Protocol):
    def area(self) -> float: ...


class Square:
    def __init__(self, s):
        self.s = s

    def area(self):
        return self.s * self.s


class Box(Generic[T]):
    def __init__(self, item):
        self.item = item

    def get(self):
        return self.item


class Movie(TypedDict):
    title: str
    year: int


def total_area(shapes):
    return sum(s.area() for s in shapes)


m = Movie(title="Dune", year=2021)
m["year"] += 1
print(total_area([Square(2), Square(3)]), Box(5).get(), Box("x").get(), UserId(42) + 1, m, m["title"])
