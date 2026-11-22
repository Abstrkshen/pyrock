from _fin_gen import gen

box = [gen("cyclic.in-list")]
next(box[0])
v = gen("cyclic.v")
next(v)


def current():
    return v
