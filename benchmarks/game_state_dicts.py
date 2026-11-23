# Breakout-style game state in dicts with mixed value types, 200k frames.
# The same loop as tests/differential/record_state_loop.py; game_state_classes.py
# holds the state in classes instead.
def new_state():
    plats = []
    for i in range(20):
        plats.append({"x": i * 50.0, "y": 400.0 - (i % 5) * 30, "w": 48.0, "h": 12.0, "kind": "ground" if i % 3 else "ledge"})
    return {"mode": "play", "time": 0, "score": 0, "ground_i": None, "plats": plats,
            "hero": {"x": 80.0, "y": 100.0, "w": 20.0, "h": 30.0, "vx": 2.5, "vy": 0.0, "ground": False}}


def overlaps(a, b):
    return a["x"] < b["x"] + b["w"] and a["x"] + a["w"] > b["x"] and a["y"] < b["y"] + b["h"] and a["y"] + a["h"] > b["y"]


def update(st):
    hero = st["hero"]
    hero["vy"] += 0.4
    hero["x"] += hero["vx"]
    hero["y"] += hero["vy"]
    hero["ground"] = False
    for p in st["plats"]:
        if overlaps(hero, p) and hero["vy"] > 0:
            hero["y"] = p["y"] - hero["h"]
            hero["vy"] = -9.0 if p["kind"] == "ledge" else 0.0
            hero["ground"] = True
            st["score"] += 1
    if hero["x"] > 1000:
        hero["x"] = 0.0
    if hero["y"] > 600:
        hero["y"] = 0.0
        hero["vy"] = 0.0
    st["time"] += 1



st = new_state()
for _ in range(200000):
    update(st)
print(st["score"], st["time"], round(st["hero"]["x"], 3), round(st["hero"]["y"], 3))
