# send / throw / close, return values, `yield from` delegation, finally.


def averager():
    total = 0.0
    count = 0
    avg = None
    while True:
        value = yield avg
        if value is None:
            break
        total += value
        count += 1
        avg = total / count
    return count


avg = averager()
print(next(avg))
print(avg.send(10), avg.send(20), avg.send(30))
try:
    avg.send(None)
except StopIteration:
    print("done")


def echo():
    try:
        while True:
            try:
                got = yield "ready"
                print("got", got)
            except ValueError as e:
                print("handled", e)
    finally:
        print("echo cleanup")


e = echo()
print(next(e))
print(e.send(1))
print(e.throw(ValueError("bad")))
e.close()
print("closed")
try:
    e.send(2)
except StopIteration:
    print("closed generator stops")


def inner(n):
    total = 0
    for i in range(n):
        x = yield i
        if x is not None:
            total += x
    return total * 10


def outer():
    r = yield from inner(3)
    print("inner returned", r)
    s = yield from inner(2)
    return r + s


o = outer()
print(next(o), o.send(5), o.send(6))
print(o.send(7))
try:
    o.send(1)
except StopIteration:
    print("outer done")


def chained():
    yield from [1, 2]
    yield from "ab"
    return "end"


def runner():
    r = yield from chained()
    yield r


print(list(runner()))


def counter():
    n = 0
    try:
        while True:
            yield n
            n += 1
    finally:
        print("counter finally at", n)


for v in counter():
    if v == 2:
        break
print("after break")
c = counter()
next(c)
next(c)
c = None
print("dropped")


def fresh():
    yield 1


f = fresh()
try:
    f.send(5)
except TypeError as err:
    print("TypeError", err)
try:
    f.throw(KeyError("k"))
except KeyError as err:
    print("KeyError", err)


def bad():
    yield 1
    raise StopIteration


try:
    list(bad())
except RuntimeError as err:
    print("RuntimeError", err)


def stubborn():
    try:
        yield 1
    except GeneratorExit:
        print("ignoring exit")
    yield 2


s = stubborn()
next(s)
try:
    s.close()
except RuntimeError as err:
    print("RuntimeError", err)
s = None  # closed again here: GeneratorExit at `yield 2` ends it
print("end")
