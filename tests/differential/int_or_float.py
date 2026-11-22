import math


def step(x, moving):
    if moving:
        x = x + math.sin(x) * 2
    return x


def total(xs):
    t = 0
    for v in xs:
        t += v
    return t


x = 760
print(x, step(x, False), step(x, True), step(3, False) * 2, step(3, True) > 3)
v = 1
for i in range(6):
    if i == 3:
        v = v / 2
    else:
        v = v * 3
    print(v, type(v).__name__ if False else "", end=" ")
print()
pts = [1, 2.5, 3, -0.5]
print(total(pts), sum(pts), max(3, 2.5), min(2, 2.5), max(2, 2.5), abs(-2.5 if pts[0] else -2))
n = 7
n = n if len(pts) > 10 else 7.0
print(n, n // 2, n % 3, -n, n ** 2, n / 2, int(n), round(n), round(2.675, 2), float(n))
k = 2
k = k if len(pts) < 10 else 2.0
print(k == 2, k == 2.0, k < 2.5, 1 < k, k ** -1, 5 // k, 5 % k, f"{k}|{k:.2f}|{k:>4}", str(k), repr(k))
grid = {"x": 0, "y": 0}
grid["x"] = grid["x"] + 0.5
grid["y"] += 2
print(grid, [step(p, i % 2 == 0) for i, p in enumerate(pts)])
try:
    print(k / (k - k))
except ZeroDivisionError as e:
    print("ZeroDivisionError", e)
print(math.floor(n + 0.5), math.sqrt(k), bool(k - k), not k)
