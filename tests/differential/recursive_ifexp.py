# Recursion through a conditional expression (`a if c else f(...)`).
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)


def depth(xs):
    return 0 if not xs else 1 + depth(xs[1:])


def collatz(n, steps):
    return steps if n == 1 else collatz(n // 2 if n % 2 == 0 else 3 * n + 1, steps + 1)


print(fib(25), depth([1, 2, 3]), collatz(27, 0))
