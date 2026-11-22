def make_counter(start):
    count = start

    def inc(step=1):
        nonlocal count
        count += step
        return count

    return inc


def make_adders():
    adders = []
    for i in range(3):
        adders.append(lambda x, i=i: x + i)
    return adders


def late_binding():
    fs = []
    for i in range(3):
        fs.append(lambda: i)
    return [f() for f in fs]


def apply_twice(f, x):
    return f(f(x))


def square(x):
    return x * x


def compose(f, g):
    return lambda x: f(g(x))


def timed(fn):
    calls = []

    def wrapper(n):
        calls.append(n)
        result = fn(n)
        print("call", len(calls), "->", result)
        return result

    return wrapper


@timed
def fib(n):
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a


c = make_counter(10)
print(c(), c(), c(5))
print([f(10) for f in make_adders()], late_binding())
print(apply_twice(square, 3), apply_twice(lambda s: s + "!", "hi"), compose(square, lambda x: x + 1)(4))
words = ["banana", "Apple", "cherry", "date"]
print(sorted(words), sorted(words, key=len), sorted(words, key=lambda w: w.lower()), sorted(words, key=len, reverse=True))
print(max(words, key=len), min(words, key=lambda w: w[-1]))
pairs = [(1, "b"), (3, "a"), (2, "c")]
pairs.sort(key=lambda p: p[1])
print(pairs, list(map(square, [1, 2, 3])), list(filter(lambda x: x % 2, range(7))), list(map(lambda a, b: a * b, [1, 2], [3, 4])))
print(fib(10), fib(5))
ops = {"add": lambda a, b: a + b, "mul": lambda a, b: a * b}
print(ops["add"](2, 3))
total = 0


def bump():
    global total
    total += 1
    return total


handlers = [bump, bump]
for h in handlers:
    h()
print(total)
