def add(a, b):
    return a + b

def twice(x):
    return add(x, x)

def square(x):
    return x * x

def sum_squares(n):
    total = 0
    for i in range(n):
        total += square(i)
    return total

def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

print(twice(12))
print(twice(1.5))
print(add("a", "b"))
print(square(2.5))
print(sum_squares(100000))
print(fib(25))
print(7 // -2, -7 % 3, 7 / 2, 2 ** 10, 1e16, 0.1 + 0.2)
