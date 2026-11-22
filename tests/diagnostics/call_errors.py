def f(a, b=1, *, c=2):
    return a + b + c


def g(items=[]):
    return items


print(f(1, d=3))
print(f(b=2))
print(f(1, 2, 3))
print(f(1, a=1))
