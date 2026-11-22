def build(n):
    out = []
    for i in range(n):
        out.append(i * i)
    return out


def fill(xs, v):
    xs.append(v)
    xs.append(v + 1)


def word_counts(text):
    counts = {}
    for w in text.split():
        w = w.lower().strip(".,!")
        counts[w] = counts.get(w, 0) + 1
    return counts


def grid(n):
    rows = []
    for r in range(n):
        row = []
        for c in range(n):
            row.append(r * n + c)
        rows.append(row)
    return rows


squares = build(6)
print(squares, len(squares), squares[0], squares[-1], squares[1:4], squares[::-2])
alias = squares
alias.append(100)
print(squares[-1], alias is squares, squares == [0, 1, 4, 9, 16, 25, 100])
items = []
fill(items, 7)
print(items)
items.insert(0, 1)
items.extend([2, 3])
print(items, items.pop(), items.pop(0), items, 7 in items, 99 not in items)
items.sort()
print(items, items.index(7), items.count(8))
items.reverse()
print(items, sorted(items), sorted(items, reverse=True), sum(items), min(items), max(items))
del items[0]
print(items)
items[0] = 42
print(items, items + [1], [0] * 3)

c = word_counts("The cat. the dog! THE end, cat")
print(c, len(c), c["the"], "cat" in c, "cow" in c)
print(list(c.keys()), list(c.values()), c.items())
for k, v in c.items():
    print(k, v)
c["cow"] = 1
del c["dog"]
print(c, c.pop("end"), c.get("zzz"), c.get("zzz", -1))
print(sorted(c), max(c.values()))

g = grid(3)
g[1][1] = -1
print(g, g[2][0])
for row in g:
    row.append(0)
print(g)

t = (1, "two", 3.0)
print(t, t[0], t[1], t[-1], len(t), t[0:2])
pair = (3, 4)
a, b = pair
print(a + b)
s = {3, 1, 2}
s.add(5)
s.discard(1)
print(len(s), 3 in s, 1 in s, sorted(s))

text = "Hello, World"
print(text[0], text[-1], text[7:], text[::-1], text.upper(), text.lower(), text.split(", "))
print(text.replace("l", "L"), text.find("World"), text.startswith("Hell"), text.endswith("x"))
print("-".join(["a", "b", "c"]), "  pad ".strip(), "x".center(5, "*"), "42".zfill(5))
print([w.capitalize() for w in ["ab", "cd"]] if False else "skip")
print(list(range(4)), list(enumerate(["x", "y"])), list(zip([1, 2], ["a", "b"])))
print(any([0, 0, 1]), all([1, 2, 0]), list(reversed([1, 2, 3])), divmod(17, 5), round(2.675, 2), round(3.5))
print(ord("A"), chr(66), repr("it's"), repr([1, "a"]), str({"k": [1, 2]}))
