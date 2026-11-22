import time


def logged(fn):
    def wrapper(*args, **kwargs):
        print("calling with", args)
        result = fn(*args, **kwargs)
        print("returned", result)
        return result

    return wrapper


def retry(times):
    def deco(fn):
        def wrapper(*args):
            last = None
            for attempt in range(times):
                try:
                    return fn(*args)
                except ValueError as e:
                    last = e
                    print("attempt", attempt + 1, "failed:", e)
            raise RuntimeError("gave up: " + str(last))

        return wrapper

    return deco


def timed(fn):
    def wrapper(*args):
        t0 = time.perf_counter()
        r = fn(*args)
        elapsed = time.perf_counter() - t0
        print("timed", elapsed >= 0)
        return r

    return wrapper


@logged
def add(a, b):
    return a + b


@logged
def greet(name, punct):
    return "hi " + name + punct


tries = {"n": 0}


@retry(3)
def flaky(x):
    tries["n"] += 1
    if tries["n"] < 3:
        raise ValueError("not yet " + str(tries["n"]))
    return x * 10


@timed
@logged
def mixed(a, b, c):
    return str(a) + b + str(c)


def total(*nums):
    s = 0
    for n in nums:
        s += n
    return s


print(add(2, 3), greet("ann", "!"))
print(flaky(4))
print(mixed(1, "-", 2.5))
print(total(*[1, 2, 3]), total(1, 2), total())
args = (5, 6)
print(add(*args))
for x in (1, "two", 3.0):
    print(x)
