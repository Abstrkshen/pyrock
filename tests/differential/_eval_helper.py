A = 1


def names():
    return sorted(k for k in globals() if not k.startswith("_"))


def calc(src):
    return eval(src)
