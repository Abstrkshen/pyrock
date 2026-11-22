# Native-friendly workload: integer and float loops, recursion.
def square(x):
    return x * x


def sum_squares(n):
    total = 0
    for i in range(n):
        total += square(i) % 1000003
    return total


def leibniz_pi(terms):
    acc = 0.0
    sign = 1.0
    for k in range(terms):
        acc += sign / (2 * k + 1)
        sign = -sign
    return 4 * acc


def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


print(sum_squares(5000000))
print(leibniz_pi(5000000))
print(fib(27))
