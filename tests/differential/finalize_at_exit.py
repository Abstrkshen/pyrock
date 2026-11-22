# Generators still open when the program ends are finalized as at CPython's
# shutdown: those held directly by module variables first, then those inside
# containers and instances (each in assignment order).


def gen(name):
    try:
        yield 1
    finally:
        print("finally", name)


class Holder:
    def __init__(self, g):
        self.g = g


h = Holder(gen("in-instance"))
next(h.g)
x = gen("plain-after")
next(x)
lst = [gen("in-list")]
next(lst[0])
state = {"g": gen("in-record"), "n": 1}
next(state["g"])
shared = gen("shared")
next(shared)


def peek():
    return shared


y = gen("plain-last")
next(y)
print("module end", peek() is shared)
