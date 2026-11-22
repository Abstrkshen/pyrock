def add(a, b):
    return a + b


def twice(x):
    return add(x, x)


def square(x):
    return x * x


def mean(a, b):
    return (a + b) / 2


def sum_squares(n):
    total = 0
    for i in range(n):
        total += square(i)
    return total


print(twice(12), twice(1.5), twice("ab"))
print(add(1, 2.5), add(True, 2), add("x", "y"))
print(square(7), square(2.5), square(True))
print(mean(1, 2), mean(1.5, 2), mean(3, 3))
print(sum_squares(1000))
