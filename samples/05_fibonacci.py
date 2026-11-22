# Recursive and iterative Fibonacci; recursion is guarded by the 1000-frame limit.
def fib_rec(n):
    if n < 2:
        return n
    return fib_rec(n - 1) + fib_rec(n - 2)


def fib_iter(n):
    a = 0
    b = 1
    for _ in range(n):
        t = a + b
        a = b
        b = t
    return a


print("fib_rec(27) =", fib_rec(27))
print("fib_iter(90) =", fib_iter(90))
for k in range(0, 15, 3):
    print(k, fib_iter(k), fib_rec(k) == fib_iter(k))
