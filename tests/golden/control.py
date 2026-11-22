def sign(n):
    if n < 0:
        return -1
    elif n == 0:
        return 0
    return 1


def count(n):
    k = 0
    while k < n:
        k += 1
        if k == 3:
            continue
    for i in range(0, 10, 3):
        k = k + i
    return k


x = 5
if 0 < x <= 10 and x != 7:
    print(sign(x), count(x))
