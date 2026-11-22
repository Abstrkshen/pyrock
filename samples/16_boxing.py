# Values whose type changes are boxed as Python objects in hybrid mode
# (and rejected with a clear error in --mode native-only).
def describe(flag):
    if flag:
        return 42
    return "forty-two"


def maybe_half(n):
    if n % 2 == 0:
        return n // 2


x = 10
print(x * 2)
x = "ten"
print(x + "!")
print(describe(True), describe(False))
print(maybe_half(8), maybe_half(7))
balance = 0
balance += 2.5
balance += 1
print(balance)
