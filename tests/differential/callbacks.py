import functools
import re


def shout(m):
    return m.group(0).upper()


def make_tally():
    seen = []

    def record(x):
        seen.append(x)
        return len(seen)

    return record, seen


@functools.lru_cache(maxsize=None)
def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


def strict(x):
    if x < 0:
        raise ValueError("negative: " + str(x))
    return x * 2


print(re.sub(r"[aeiou]", shout, "native callbacks"))
print(re.sub(r"\d+", lambda m: str(int(m.group(0)) * 10), "a1 b22 c3"))
print(functools.reduce(lambda a, b: a * b, [1, 2, 3, 4, 5]))
record, seen = make_tally()
print(list(map(record, ["x", "y"])), seen)
print(fib(60), fib.cache_info().hits > 0)
print(sorted(["bb", "a", "ccc"], key=lambda s: -len(s)))
try:
    print(list(map(strict, [1, -2])))
except ValueError as e:
    print("ValueError:", e)
