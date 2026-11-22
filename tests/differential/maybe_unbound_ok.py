def tail(n):
    for i in range(n):
        last = i * 2
    if n > 0:
        return last
    return -1


print(tail(5), tail(0))
