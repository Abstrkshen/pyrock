import itertools


def count_up(n):
    i = 0
    while i < n:
        yield i
        i += 1


def naturals():
    n = 0
    while True:
        yield n
        n += 1


def evens(xs):
    for x in xs:
        if x % 2 == 0:
            yield x


def chain(a, b):
    yield from a
    yield from b


def words(text):
    for w in text.split():
        yield w.upper()
    return


def nothing():
    if False:
        yield 1


def fib():
    a, b = 0, 1
    while True:
        yield a
        a, b = b, a + b


class Tree:
    def __init__(self, value, children):
        self.value = value
        self.children = children

    def walk(self):
        yield self.value
        for c in self.children:
            yield from c.walk()


def pairs(n):
    for i in range(n):
        yield (i, i * i)


def noisy():
    print("start")
    yield 1
    print("middle")
    yield 2
    print("end")


print(list(count_up(5)))
total = 0
for n in naturals():
    if n > 100:
        break
    total += n
print(total)
print(sum(evens(range(10))), list(chain([1, 2], count_up(3))))
print(list(words("hello big world")), list(nothing()))
for i, f in enumerate(fib()):
    if i == 10:
        print("fib10", f)
        break
t = Tree(1, [Tree(2, [Tree(4, [])]), Tree(3, [])])
print(list(t.walk()))
print(dict(pairs(4)), [a + b for a, b in pairs(3)])
g = noisy()
print("created")
for v in g:
    print("got", v)
print(list(g))
print(sorted(count_up(4), reverse=True), max(count_up(7)))
print(list(itertools.islice(naturals(), 3)))
gen = count_up(3)
print(next(gen), next(gen))
print(next(gen, -1), next(gen, -1))
try:
    next(gen)
except StopIteration:
    print("stop")
