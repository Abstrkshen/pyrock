import itertools

xs = [3, 1, 4, 1, 5, 9, 2, 6]
print(sum(x * x for x in xs), max(x for x in xs if x % 2), min((x for x in xs), default=0))
print(list(x + y for x in range(3) for y in range(x)))
grid = [[1, 2], [3, 4], [5]]
print([sum(v for v in row) for row in grid])
offset = 10
g = (x + offset for x in xs)
offset = 100
print(next(g), list(g))
print(any(x > 8 for x in xs), all(x > 0 for x in xs), "-".join(str(x) for x in xs))


def naturals():
    n = 0
    while True:
        yield n
        n += 1


squares = (n * n for n in naturals())
for s in squares:
    if s > 50:
        print("first square > 50:", s)
        break
log = []


def noisy(v):
    log.append(v)
    return v


lazy = (noisy(x) for x in [1, 2, 3])
print(log)
print(next(lazy), log)
print(sorted(set(c for c in "mississippi")), dict((k, len(k)) for k in ["a", "bb"]))
print(list(itertools.islice((x * 3 for x in naturals()), 4)))
print(tuple(w.upper() for w in "a b c".split()))


def total(it):
    s = 0
    for v in it:
        s += v
    return s


print(total(x for x in xs if x > 2))
print(max((x for x in [] if x), default=-1), min([], default="none"), max([2, 7], default=0))
