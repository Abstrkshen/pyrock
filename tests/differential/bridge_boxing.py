def choose(flag):
    if flag:
        return 42
    return "hello"


def maybe(n):
    if n > 0:
        return n * 2


x = 1
print(x + 1)
x = "now a string"
print(x + "!")
print(choose(True), choose(False), maybe(3), maybe(-1))
total = 0
total += 0.5
total += 1
print(total)
y = 3 if x else "no"
print(y, 1 or "s", 0 or "s", min(1, 2.5), max(1, 2.5))
