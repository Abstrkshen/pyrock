# Lists and dicts are real Python objects, so aliasing behaves exactly as in Python.
def append_twice(items, value):
    items.append(value)
    items.append(value)


a = [1, 2]
b = a              # same list, not a copy
b.append(3)
append_twice(a, 9)
print(a, b, a is b)

c = a
c += [4]           # in-place: mutates the list `a` refers to
print(a, c is a)

t = (1, 2)
u = t
t += (3,)          # tuples are immutable: `t` is rebound, `u` is unchanged
print(t, u)

grid = {"row": [0, 0, 0]}
row = grid["row"]
row[1] = 5
grid["size"] = len(row)
print(grid)
