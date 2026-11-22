# Integer and float arithmetic with Python semantics.
print(7 // 2, -7 // 2, 7 // -2, -7 // -2)
print(7 % 3, -7 % 3, 7 % -3, -7 % -3)
print(7 / 2, -7 / 2, 1 / 3, 10 / 5)
print(2 ** 10, 2 ** 0, (-3) ** 3, 2 ** -1, 2.0 ** 0.5)
print(7.5 // 2, -7.5 // 2, 7.5 % 2, -7.5 % 2, 7.5 % -2)
print(-0.0 % 1.0, 0.0 % -1.0)
print(1 + 2.5, 3 * 0.5, 10 - 0.25)
print(True + True, True * 3, -True, +False, ~5, ~True)
print(6 & 3, 6 | 3, 6 ^ 3, True & False, True | False, 1 << 10, -16 >> 2, 5 >> 100, -5 >> 100)
print(abs(-5), abs(-2.5), abs(True))
print(min(3, 1, 2), max(3, 1, 2), min(2.5, 1.5), max("a", "b"), max(1.0, 1.0))
print(9007199254740993 == 9007199254740992.0, 9007199254740993 > 9007199254740992.0)
print(1 < 2 < 3, 3 > 2 > 2, 1 == 1.0, 1 != 1.0, 2 < 2.5, 2.5 <= 2)
print(1 == "1", "a" != 1, None == None)
print(int(3.9), int(-3.9), int("  -42 "), int("1_000"), int(True), int())
print(float(3), float("2.5"), float(" -inf "), float("1e-3"), float(False))
print(str(12), str(2.0), str(True), str(None), bool(0), bool(0.0), bool("x"), bool(""))
print(1e16, 1e15, 1.5e-5, 0.0001, 123456789.125, -0.0, 0.1 + 0.2, 1 / 7)
big = 1e308 * 10
print(big, -big, big - big)
x = 10
x += 5
x *= 2
x -= 1
x //= 4
x %= 5
x **= 3
print(x)
y = 1.5
y /= 2
print(y)
print(len("héllo"), len(""))
print(f"x={x} y={y} ok={x > 1}")
