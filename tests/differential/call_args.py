def greet(name, greeting="Hello", punct="!"):
    return greeting + ", " + name + punct


def scaled(x, factor=2, *, offset=0, clamp=None):
    v = x * factor + offset
    if clamp is not None and v > clamp:
        v = clamp
    return v


def pos_only(a, b, /, c=3):
    return a * 100 + b * 10 + c


def trace(label, value):
    print("eval", label)
    return value


def pair(first, second):
    return first - second


def window(n, size=None):
    if size is None:
        size = n // 2
    return n - size


print(greet("Ada"))
print(greet("Bob", "Hi"))
print(greet("Cy", punct="?"))
print(greet(punct=".", name="Dee", greeting="Hey"))
print(scaled(5), scaled(5, 3), scaled(5, offset=1), scaled(5, clamp=7), scaled(2.5, factor=2, offset=-1))
print(pos_only(1, 2), pos_only(1, 2, c=9), pos_only(4, 5, 6))
print(pair(second=trace("second", 1), first=trace("first", 10)))
print(window(10), window(10, 3), window(10, size=1))
print("a", "b", "c", sep="-")
print("no newline", end="")
print(" ... continued", 1, 2.5, sep="|", end="\n")
print("x", sep=None, end=None)
print("flushed", flush=True)
