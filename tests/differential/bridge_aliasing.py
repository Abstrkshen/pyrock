def push(lst, v):
    lst.append(v)


a = [1, 2]
b = a
b.append(3)
push(a, 4)
print(a, b, a is b)
c = a
c += [5]
print(a, c is a)
d = {"k": [1]}
inner = d["k"]
inner.append(2)
d["n"] = len(inner)
print(d)
t = (1, 2)
u = t
t += (3,)
print(t, u)
