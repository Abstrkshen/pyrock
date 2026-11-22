import json
import random


def shuffled(v):
    random.shuffle(v)
    return v


def tally(rows):
    out = {}
    for name, score in rows:
        out.setdefault(name, []).append(score)
    return out


random.seed(7)
xs = [1, 2, 3, 4, 5]
ys = xs
random.shuffle(xs)
print(xs, ys, xs is ys)

a = [3, 1, 2, 9, 8]
b = shuffled(a)
print(a is b, a == b, sorted(a))

nested = {"k": [1, 2]}
inner = nested["k"]
inner.append(3)
print(nested, json.dumps(nested))

mixed = [1, "a", 2.5, None]
mixed.append([1])
print(mixed, len(mixed))

data = json.loads("[1, 2]")
data.append(3)
print(data)

t = tally([("ann", 3), ("bob", 5), ("ann", 4)])
print(t, sorted(t.items()))

first, *rest = [10, 20, 30]
print(first, rest)
(p, q), r = (1, 2), 3
print(p, q, r)
x, y = y_x = (5, 6)
print(x, y, y_x)
try:
    m, n = [1, 2, 3]
except ValueError as e:
    print("ValueError:", e)
squares = {n: n * n for n in range(5) if n % 2 == 0}
print(squares, {w for w in "abca"} == {"a", "b", "c"}, sum(i for i in range(10)))
pairs = [(i, j) for i in range(3) for j in range(i)]
print(pairs)
print(n)
