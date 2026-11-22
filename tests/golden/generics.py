def add(a, b):
    return a + b


def twice(x):
    return add(x, x)


def sum_squares(n):
    total = 0
    for i in range(n):
        total += i * i
    return total


print(twice(12))
print(twice(1.5))
print(add("a", "b"))
print(sum_squares(100))
