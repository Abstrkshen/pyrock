# Importing a Rust file: every simple `pub fn` in fastmath.rs is callable.
import fastmath
from fastmath import fib, safe_div


def dist(x1, y1, x2, y2):
    return fastmath.hypot(x2 - x1, y2 - y1)


print("hypot:", dist(0, 0, 3, 4), dist(1.5, 2.0, 4.5, 6.0))
print("fib(80):", fib(80))
print("primes below 1e6:", fastmath.count_primes(1000000))
print(fastmath.shout("rust says hi"))
print("safe_div:", safe_div(10, 3))
print("safe_div by zero:", safe_div(1, 0))
