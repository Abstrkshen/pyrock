# @dataclass options: frozen, order, eq, repr, kw_only, slots.
from dataclasses import dataclass, field


@dataclass(frozen=True)
class Point:
    x: int
    y: int


@dataclass(order=True)
class Version:
    major: int
    minor: int = 0
    tag: str = ""


@dataclass(eq=False, repr=False)
class Handle:
    name: str


@dataclass(kw_only=True, slots=True)
class Config:
    host: str
    port: int = 8080
    tags: list = field(default_factory=list)


p = Point(1, 2)
print(p, p == Point(1, 2), {p: "a", Point(1, 2): "b"}, len({p, Point(1, 2), Point(2, 1)}))
try:
    p.x = 5
except AttributeError as e:
    print(type(e).__name__, e)
vs = [Version(1, 2), Version(1, 0, "b"), Version(0, 9), Version(1, 0, "a")]
print(sorted(vs))
print(Version(1, 2) < Version(1, 10), Version(2) > Version(1, 99), max(vs))
h = Handle("x")
print(h == Handle("x"), h == h, repr(h).startswith("<__main__.Handle object at 0x"))
c = Config(host="localhost")
c.tags.append("a")
print(c, Config(host="h", port=1))
