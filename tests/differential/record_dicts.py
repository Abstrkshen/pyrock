# Dicts with fixed string keys compile to structs; every other use must behave
# exactly like a dict.
import json


def make_hero(x):
    return {"x": x, "y": 0.0, "name": "lumen", "lives": 3, "target": None}


h = make_hero(1.5)
alias = h
alias["x"] += 2.0
print(h["x"], h is alias, h == alias, len(h), "x" in h, "z" in h, 5 in h)
print(h)
print(str(h), repr({"it's": 1, 'say "hi"': 2}))
h["target"] = 7
print(h["target"], h.get("lives"), h.get("lives", 0), h.get("missing", "dflt"))
print(sorted(h), list(h), [k for k in h], isinstance(h, dict))
print(h == make_hero(3.5), h != make_hero(1.5))
print(json.dumps({"a": 1, "b": [1, 2], "c": {"d": True}}))

# Records inside containers, nested records.
stars = []
for i in range(3):
    stars.append({"x": i * 10, "y": i * 2, "spin": i * 0.5})
total = 0
for s in stars:
    s["x"] += 1
    total += s["x"] + s["y"]
print(total, stars[1])
state = {"hero": make_hero(0.0), "stars": stars, "time": 0}
state["hero"]["lives"] -= 1
state["time"] += 1
print(state["hero"]["lives"], state["time"], len(state["stars"]))

# Uses that make it a plain dict.
d = {"a": 1, "b": 2}
d["c"] = 3
print(d, len(d))
e = {"a": 1, "b": 2}
for k, v in e.items():
    print(k, v)
key = "b"
f = {"a": 1, "b": 2}
print(f[key], f.get(key))
g = {"a": 1, "b": 2}
del g["a"]
print(g)
m = {"x": 1, "y": "two"}
m.update({"z": 3.0})
print(m, sorted(m.keys()), list(m.values()))
c = {"p": 1}
c2 = c.copy()
c2["p"] = 5
print(c, c2)

# Escaping to CPython: a function that mutates it.
def mutate(obj):
    obj["added"] = True


import types

ns = types.SimpleNamespace(f=mutate)
r = {"k": 1}
ns.f(r)
print(r)

# Field types widen; same keys with different types join per key.
w = {"v": None, "n": 0}
w["v"] = "set"
print(w)
pts = [{"x": 1, "y": 2}, {"x": 1.5, "y": 2}]
print(pts[0]["x"] + pts[1]["x"])
shapes = [{"kind": "sq", "side": 2}, {"kind": "circle", "r": 1.0}]
print([s["kind"] for s in shapes], shapes[1])

try:
    print(h["nope"])
except KeyError as ex:
    print("KeyError", ex)

# Keys added after creation stay native (optional fields).
hero = {"x": 1.0, "vy": 0.0}
print("prev_bottom" in hero, hero.get("prev_bottom"), hero.get("prev_bottom", -1.0), len(hero))
hero["prev_bottom"] = hero["x"] + 5
hero["phase"] = "air"
print(hero, len(hero), "phase" in hero, hero["prev_bottom"], hero.get("phase", "?"))
other = {"x": 2.0, "vy": 1.0}
other["phase"] = "ground"
other["prev_bottom"] = 3.0
print(other, sorted(other), list(other))
del hero["phase"]
print(hero, "phase" in hero)
try:
    del hero["phase"]
except KeyError as ex:
    print("KeyError", ex)
try:
    print(other["missing"])
except KeyError as ex:
    print("KeyError", ex)
print(hero == {"x": 1.0, "vy": 0.0, "prev_bottom": 6.0}, hero == other)
print(json.dumps(other))

# Records of different shapes in one place: a shared prefix stays required,
# the other keys are optional, in each value's own order.
plats = [
    {"x": 0, "y": 400, "w": 100, "kind": "ground"},
    {"x": 760, "y": 400, "w": 150, "kind": "stone", "mx": 150, "speed": 0.025},
    {"x": 5, "kind": "odd", "y": 1, "w": 2},
]
for p in plats:
    print(p, len(p), "mx" in p, p.get("speed", 0.0), list(p))
moving = [p for p in plats if "mx" in p]
print(moving[0]["mx"] * 2, sum(p["w"] for p in plats))
a = {"k": 1, "v": "x"}
b = {"v": "y", "k": 2}
pair = [a, b]
print(pair, [sorted(q) for q in pair])
