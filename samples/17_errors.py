# Uncaught exceptions end the program like CPython: same type, message, exit status 1.
def safe_ratio(a, b):
    if b == 0:
        return 0.0
    return a / b


def ratio(a, b):
    return a // b


print(safe_ratio(1, 0), safe_ratio(7, 2))
print(ratio(17, 5))
print(int("123") + 1)
print(ratio(1, 0))
print("never printed")
