def classify(n):
    if n < 0:
        return "negative"
    elif n == 0:
        return "zero"
    elif n < 10:
        return "small"
    else:
        return "large"


def collatz(n):
    steps = 0
    while n != 1:
        if n % 2 == 0:
            n = n // 2
        else:
            n = 3 * n + 1
        steps += 1
    return steps


def first_multiple(k, limit):
    found = -1
    for i in range(1, limit):
        if i % k != 0:
            continue
        found = i
        break
    return found


def count_down():
    out = 0
    for i in range(10, 0, -3):
        out = out * 10 + i
    return out


def last_index(n):
    for i in range(n):
        pass
    return i


def spin():
    k = 0
    while True:
        k += 1
        if k > 5:
            break
    return k


def nested(n):
    total = 0
    for i in range(n):
        for j in range(i):
            if (i + j) % 3 == 0:
                continue
            total += i * j
    return total


print(classify(-5), classify(0), classify(7), classify(100))
print(collatz(27))
print(first_multiple(7, 100), first_multiple(200, 100))
print(count_down())
print(last_index(5))
print(spin())
print(nested(30))
print(1 if 2 > 1 else 0, "yes" if "" else "no")
