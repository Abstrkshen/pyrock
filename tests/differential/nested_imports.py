# Imports inside functions and blocks: CPython imports that bind variables.
try:
    import module_that_does_not_exist as missing
except ImportError:
    missing = None

try:
    from json import dumps
except ImportError:
    dumps = None

if missing is None:
    import math as m
    ROOT2 = m.sqrt(2.0)
else:
    ROOT2 = 0.0


def lazy(xs):
    import json
    from statistics import mean
    import os.path
    return json.dumps(xs) + " " + str(mean(xs)) + " " + os.path.basename("/a/b.txt")


def lazy_alias():
    import json as j
    from collections import Counter as C
    return j.loads("[1, 2]"), C("abca").most_common(1)


def import_error():
    try:
        from json import no_such_name
    except ImportError as e:
        return "ImportError: " + str(e)
    return "imported " + str(no_such_name)


def counter():
    total = 0
    for i in range(3):
        import functools
        total += functools.reduce(lambda a, b: a + b, [i, i, 1])
    return total


print(missing is None, dumps({"a": 1}), round(ROOT2, 6))
print(lazy([1, 2, 3]))
print(lazy_alias())
print(import_error())
print(counter())
