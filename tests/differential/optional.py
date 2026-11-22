def maybe_half(n):
    if n % 2 == 0:
        return n // 2


def find(word, letter):
    i = 0
    while i < len(word):
        if word[i:i + 1] == letter:
            return i
        i += 1
    return None


def describe(x):
    if x is None:
        return "nothing"
    return "value " + str(x * 2)


def total(a, b):
    r = None
    if a > 0:
        r = a
    if b > 0:
        if r is None:
            r = 0
        r = r + b
    return r


for n in range(4):
    h = maybe_half(n)
    print(n, h, h is None, h == 1, h != None, describe(h))
print(find("hello", "l"), find("hello", "z"))
print(total(1, 2), total(0, 3), total(0, 0), total(5, 0))
v = maybe_half(4)
print(v + 1, -v, v < 3, abs(v), float(v), str(v))
w = maybe_half(3)
print(bool(w), w or 7, w is not None)
print(w + 1)
