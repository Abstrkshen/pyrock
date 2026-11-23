# mode: native-only
# Generator expressions passed to builtins: eager consumers compile to loops,
# `any` / `all` stop at the first deciding item, `next` reads one item.


def noisy(x):
    print("saw", x)
    return x > 1


def count():
    i = 0
    while True:
        yield i
        i += 1


words = ["alpha", "beta", "gamma"]
print(tuple(ord(c) for c in "ab "))
print(list(len(w) for w in words), sorted(set(w[0] for w in words)))
print(sorted((w for w in words), key=len, reverse=True))
print("-".join(w.upper() for w in words if "a" in w))
print(sum(len(w) for w in words), sum(x * 0.5 for x in [1, 2, 3]))
print(min(len(w) for w in words), max((w for w in words), key=len))
print(max((x for x in [] if x), default=-1))
print(dict((w, len(w)) for w in words))

# Short-circuit: only the items up to the deciding one are computed.
print(any(noisy(x) for x in [1, 2, 3]))
print(all(noisy(x) for x in [2, 0, 5]))
print(all(x < 3 for x in count()), any(x > 4 for x in count()))
print(next(x for x in count() if x * x > 50))

# A generator expression kept in a variable stays lazy.
g = (noisy(x) for x in [5, 6, 7])
print(next(g))
print(list(g))


# Eager consumers inside a loop (one coroutine per iteration before).
total = 0
for w in words * 1000:
    total += len(tuple(ord(c) for c in w)) + sum(ord(c) for c in w)
print(total)
