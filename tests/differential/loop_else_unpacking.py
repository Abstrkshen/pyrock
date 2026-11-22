# for/while ... else, `...` placeholders, and * / ** unpacking in literals.
class Protocol:
    def run(self):
        ...


def find(xs, target):
    for i, x in enumerate(xs):
        if x == target:
            print("found at", i)
            break
    else:
        print("not found")


def first_square_over(limit):
    n = 0
    while n < 100:
        n += 1
        if n * n > limit:
            break
    else:
        return -1
    return n


def nested(rows):
    for r in rows:
        for x in r:
            if x < 0:
                break
        else:
            continue
        print("negative in", r)
        break
    else:
        print("all rows non-negative")


def stub():
    ...


a = [1, 2]
b = (3, 4)
print([*a, *b, 5], (*a, 0), {*a, 2, 7} == {1, 2, 7}, [*range(3)])
d = {"x": 1, "y": 2}
print({**d, "y": 3, "z": 4}, {"w": 0, **d}, {**d})
find([3, 5, 7], 5)
find([3, 5, 7], 4)
print(first_square_over(50), first_square_over(1000000))
nested([[1, 2], [3, -1], [4]])
nested([[1], [2]])
print(stub(), Protocol().run())
