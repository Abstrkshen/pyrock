import json


def total(*nums):
    s = 0
    for n in nums:
        s += n
    return s


def describe(name, *tags, sep=", ", **attrs):
    parts = [name]
    parts.append(sep.join(tags))
    for k, v in attrs.items():
        parts.append(k + "=" + str(v))
    return " | ".join(parts)


def head_tail(first, *rest):
    return first, rest, len(rest)


print(total(), total(1), total(1, 2, 3))
print(describe("box"), describe("box", "red", "big", size=3, weight=7), describe("x", "a", sep="/"))
xs = [4, 5, 6]
print(total(*xs), total(1, *xs), head_tail(1, 2, 3), head_tail(0, *xs))
print(*xs)
print(*xs, sep="-")
print("start", *["a", "b"], "end")
opts = {"indent": 2, "sort_keys": True}
print(json.dumps({"b": 1, "a": [1, 2]}, **opts))
args = ([1, 2],)
print(json.dumps(*args))
