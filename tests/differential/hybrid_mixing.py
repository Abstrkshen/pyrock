# Native values next to CPython objects (found by examples/apps/backend).
import json

from _shapes import Square, area_sum

data = json.loads('{"a": 1, "b": 2, "a2": 3}')
counts = {}
for k in data:
    counts[k[0]] = counts.get(k[0], 0) + data[k]
print(sorted(counts.items()))
parts = [p for p in json.loads('"x/y"').split("/") if p]
print(parts == ["x", "y"], ["x", "y"] == parts, parts != ["x"], (json.loads("1"), 2) == (1, 2))
words = json.loads('["b", "a"]')
print(words[json.loads("0")], "ab"[json.loads("1")])


def find(tags, tag=None):
    return tag is not None and tag in tags


print(find(["a", "b"]), find(["a"], "a"), find(["a"], "z"))
s = Square(3)
print(s.area(), area_sum([Square(1), Square(2)]))


def always_fails(msg):
    raise ValueError(msg)


try:
    always_fails("nope")
except ValueError as e:
    print("caught", e)
