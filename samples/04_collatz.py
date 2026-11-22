# Longest Collatz chain below a limit: tight integer loops, floor division.
def collatz_length(n):
    steps = 1
    while n != 1:
        if n % 2 == 0:
            n = n // 2
        else:
            n = 3 * n + 1
        steps += 1
    return steps


best = 0
best_start = 0
for start in range(1, 300000):
    length = collatz_length(start)
    if length > best:
        best = length
        best_start = start
print("longest chain below 300000 starts at", best_start, "with", best, "terms")
