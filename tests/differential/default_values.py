# Non-literal defaults are evaluated once, where the def runs (shared mutable
# default, side effects in definition order), for functions and methods.
import os

SEP = "/"
_sentinel = object()
calls = []


def tag(n):
    calls.append(n)
    return n * 10


def join(a, b, sep=SEP):
    return a + sep + b


def acc(x, into=[]):
    into.append(x)
    return into


def once(v=tag(1)):
    return v


def pick(d, key, default=_sentinel):
    if key in d:
        return d[key]
    if default is _sentinel:
        raise KeyError(key)
    return default


def path(p, sep=os.sep, n=len("abc") + 1):
    return p.replace("/", sep) * n


class Box:
    LIMIT = 3

    def __init__(self, items=None, cap=tag(2)):
        self.items = items if items is not None else []
        self.cap = cap

    def add(self, x, scale=SEP * 2):
        self.items.append(str(x) + scale)
        return self.items


SEP = "|"
print(join("a", "b"), join("a", "b", "-"))
print(acc(1), acc(2), acc(3, []))
print(once(), once(), calls)
print(pick({"a": 1}, "a"), pick({}, "b", 5))
try:
    pick({}, "zz")
except KeyError as e:
    print("KeyError", e)
print(path("x/y"))
b = Box()
print(b.cap, b.add(1), Box([0]).add(2, "!"))


# A dict literal that never receives a key, only read from; a Python object
# passed where a native str method needs a str.
def lookup(d, key, default):
    if key in d:
        return d[key]
    return default


def lookup2(d, key):
    return d[key]


def native_sep(p, sep):
    return p.replace("/", sep).strip(sep)


print(lookup({}, "b", 5), native_sep("/x/y/", os.sep))
try:
    print(lookup({}, "b", 5), lookup({}, "c", 6) + lookup({"c": 1}, "c", 0))
    print(lookup2({}, "zz"))
except KeyError as e:
    print("KeyError", e)
