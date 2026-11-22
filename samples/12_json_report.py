# Native computation feeding the Python standard library (json is pure Python).
import json


def stats(n):
    total = 0
    squares = 0
    for i in range(1, n + 1):
        total += i
        squares += i * i
    return total, squares


def mean(n):
    return (n + 1) / 2


report = {
    "n": 1000,
    "sum": stats(1000)[0],
    "sum_of_squares": stats(1000)[1],
    "mean": mean(1000),
    "tags": ["native", "bridge"],
}
text = json.dumps(report, sort_keys=True)
print(text)
back = json.loads(text)
print(back["sum"] + 1, back["tags"][0], len(back))
