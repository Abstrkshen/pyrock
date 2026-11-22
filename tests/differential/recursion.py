def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


def ackermann(m, n):
    if m == 0:
        return n + 1
    if n == 0:
        return ackermann(m - 1, 1)
    return ackermann(m - 1, ackermann(m, n - 1))


def is_even(n):
    if n == 0:
        return True
    return is_odd(n - 1)


def is_odd(n):
    if n == 0:
        return False
    return is_even(n - 1)


def gcd(a, b):
    while b:
        t = a % b
        a = b
        b = t
    return a


print(fib(20), fib(1), fib(0))
print(ackermann(2, 3))
print(is_even(10), is_odd(7), is_even(7))
print(gcd(1071, 462), gcd(17, 5))
