class A:
    def __init__(self):
        self.xs = [1, 2]


class B:
    def __init__(self):
        self.n = 1


def f(v):
    if isinstance(v, A):
        for i in range(len(v.xs)):
            print(v.xs[i])


f(A())
f(B())
