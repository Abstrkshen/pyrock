import json
import math
from collections import Counter, OrderedDict


def mean(values):
    total = 0.0
    for v in values:
        total += v
    return total / len(values)


data = json.loads('{"xs": [3, 1, 4, 1, 5, 9, 2, 6], "name": "pi"}')
xs = data["xs"]
print(data["name"], len(xs), sorted(xs), max(xs), sum(xs))
print(round(mean(xs), 3), math.floor(mean(xs)), math.pi)
print(json.dumps({"mean": mean(xs), "ok": True, "none": None}, sort_keys=True))
c = Counter("mississippi")
print(c.most_common(3))
od = OrderedDict()
od["z"] = 1
od["a"] = 2
print(list(od.keys()), "z" in od, "q" not in od)
print("a", end="")
print("b", 1, sep="-")
print(f"{math.pi:.3f} {len(xs)!r} {'x':>3}")
print(xs[1:4], xs[::-1][0], xs[-1])
