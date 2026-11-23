import math

import rl

import svgload

SCREEN_W = 960
SCREEN_H = 540
GROUND = 472
SKY = (12, 14, 36, 255)
DUSK = (42, 28, 68, 255)
GOLD = (255, 213, 106, 255)
INK = (246, 242, 255, 255)
DIM = (186, 180, 214, 255)
PANEL = (12, 14, 32, 232)
LOCKED = (150, 150, 168, 255)

LEVELS = [
    {
        "name": "Cinder Path",
        "blurb": "Carry the lantern",
        "detail": "across the first field.",
        "width": 2100,
        "spawn": (80, 396),
        "portal": (1960, 352, 78, 120),
        "platforms": [
            {"x": 0, "y": GROUND, "w": 2100, "h": 140, "kind": "ground"},
            {"x": 460, "y": 360, "w": 180, "h": 22, "kind": "stone"},
            {"x": 860, "y": 300, "w": 160, "h": 22, "kind": "stone"},
            {"x": 1240, "y": 360, "w": 200, "h": 22, "kind": "stone"},
            {"x": 1600, "y": 320, "w": 170, "h": 22, "kind": "stone"},
        ],
        "moths": [
            {"x": 560, "y": 300, "minx": 480, "maxx": 780, "speed": 1.5},
            {"x": 1320, "y": 390, "minx": 1180, "maxx": 1540, "speed": 1.7},
        ],
        "beetles": [
            {"x": 980, "y": 434, "minx": 900, "maxx": 1200, "speed": 1.25},
        ],
        "wisps": [(520, 300), (930, 240), (1660, 260), (1860, 400)],
    },
    {
        "name": "Moth Gallery",
        "blurb": "The gallery is full",
        "detail": "of wings and gaps.",
        "width": 2460,
        "spawn": (70, 396),
        "portal": (2280, 352, 78, 120),
        "platforms": [
            {"x": 0, "y": GROUND, "w": 520, "h": 140, "kind": "ground"},
            {"x": 700, "y": GROUND, "w": 460, "h": 140, "kind": "ground"},
            {"x": 1320, "y": GROUND, "w": 420, "h": 140, "kind": "ground"},
            {"x": 1900, "y": GROUND, "w": 560, "h": 140, "kind": "ground"},
            {"x": 490, "y": 378, "w": 170, "h": 22, "kind": "stone"},
            {"x": 1100, "y": 340, "w": 180, "h": 22, "kind": "stone"},
            {"x": 1680, "y": 360, "w": 180, "h": 22, "kind": "stone"},
        ],
        "moths": [
            {"x": 560, "y": 300, "minx": 500, "maxx": 760, "speed": 1.8},
            {"x": 1160, "y": 270, "minx": 1080, "maxx": 1360, "speed": 2.0},
            {"x": 1760, "y": 300, "minx": 1660, "maxx": 1980, "speed": 1.8},
        ],
        "beetles": [
            {"x": 820, "y": 434, "minx": 740, "maxx": 1080, "speed": 1.4},
            {"x": 2000, "y": 434, "minx": 1940, "maxx": 2240, "speed": 1.5},
        ],
        "wisps": [(560, 310), (1180, 270), (1740, 290), (2140, 390)],
    },
    {
        "name": "The Spire",
        "blurb": "The last light sits",
        "detail": "above the moving stone.",
        "width": 2760,
        "spawn": (70, 396),
        "portal": (2520, 352, 78, 120),
        "platforms": [
            {"x": 0, "y": GROUND, "w": 640, "h": 140, "kind": "ground"},
            {"x": 1040, "y": GROUND, "w": 500, "h": 140, "kind": "ground"},
            {"x": 1760, "y": GROUND, "w": 1000, "h": 140, "kind": "ground"},
            {"x": 760, "y": 400, "w": 150, "h": 22, "kind": "stone", "mx": 150, "speed": 0.025},
            {"x": 1500, "y": 360, "w": 180, "h": 22, "kind": "stone"},
            {"x": 1960, "y": 360, "w": 150, "h": 22, "kind": "stone"},
            {"x": 2180, "y": 280, "w": 140, "h": 22, "kind": "stone"},
            {"x": 2400, "y": 360, "w": 160, "h": 22, "kind": "stone"},
        ],
        "moths": [
            {"x": 400, "y": 340, "minx": 260, "maxx": 600, "speed": 2.1},
            {"x": 1200, "y": 360, "minx": 1080, "maxx": 1460, "speed": 2.2},
            {"x": 2100, "y": 240, "minx": 1960, "maxx": 2360, "speed": 1.9},
        ],
        "beetles": [
            {"x": 1180, "y": 434, "minx": 1080, "maxx": 1460, "speed": 1.6},
            {"x": 1900, "y": 434, "minx": 1800, "maxx": 2140, "speed": 1.7},
        ],
        "wisps": [(820, 340), (1560, 300), (2240, 220), (2460, 300), (2360, 400)],
    },
]


def current(st):
    return LEVELS[st["level_i"]]


def new_hero(spawn):
    return {
        "x": float(spawn[0]),
        "y": float(spawn[1]),
        "w": 34.0,
        "h": 76.0,
        "vx": 0.0,
        "vy": 0.0,
        "face": 1,
        "ground": False,
        "coyote": 0,
        "jump_buf": 0,
        "squash": 0,
    }


def new_state():
    stars = []
    i = 0
    while i < 50:
        stars.append({"x": (i * 149) % SCREEN_W, "y": (i * 67) % 250, "p": i * 0.7})
        i += 1
    return {
        "mode": "menu",
        "menu_i": 0,
        "level_i": 0,
        "card": 0,
        "pause_i": 0,
        "score": 0,
        "mark": 0,
        "lives": 3,
        "time": 0,
        "banner": 0,
        "invuln": 0,
        "shake": 0,
        "ground_i": None,
        "plats": [],
        "hero": new_hero((80, 396)),
        "moths": [],
        "beetles": [],
        "wisps": [],
        "sparks": [],
        "quit": False,
        "auto": False,
        "stars": stars,
    }


def begin_level(st, index, keep_mark):
    src = LEVELS[index]
    mark = st["mark"] if keep_mark else st["score"]
    st["mode"] = "play"
    st["level_i"] = index
    st["mark"] = mark
    st["score"] = mark
    st["lives"] = 3
    st["hero"] = new_hero(src["spawn"])
    st["moths"] = []
    for i, moth in enumerate(src["moths"]):
        st["moths"].append({
            "x": float(moth["x"]),
            "y": float(moth["y"]),
            "minx": moth["minx"],
            "maxx": moth["maxx"],
            "speed": moth["speed"],
            "dir": 1,
            "phase": i * 1.3,
            "alive": True,
            "w": 56.0,
            "h": 34.0,
        })
    st["beetles"] = []
    for i, bug in enumerate(src["beetles"]):
        st["beetles"].append({
            "x": float(bug["x"]),
            "y": float(bug["y"]),
            "minx": bug["minx"],
            "maxx": bug["maxx"],
            "speed": bug["speed"],
            "dir": 1 if i % 2 == 0 else -1,
            "alive": True,
            "w": 50.0,
            "h": 36.0,
        })
    st["wisps"] = []
    for i, spot in enumerate(src["wisps"]):
        st["wisps"].append({"x": float(spot[0]), "y": float(spot[1]), "got": False, "phase": i * 0.9})
    st["sparks"] = []
    st["invuln"] = 0
    st["shake"] = 0
    st["banner"] = 160
    st["ground_i"] = None
    st["plats"] = []
    st["pause_i"] = 0


def bake_platforms(st):
    baked = []
    for plat in current(st)["platforms"]:
        x = plat["x"]
        y = plat["y"]
        if "mx" in plat:
            x = x + math.sin(st["time"] * plat["speed"]) * plat["mx"]
        baked.append({"x": x, "y": y, "w": plat["w"], "h": plat["h"], "kind": plat["kind"]})
    return baked


def overlaps(a, b):
    return a["x"] < b["x"] + b["w"] and a["x"] + a["w"] > b["x"] and a["y"] < b["y"] + b["h"] and a["y"] + a["h"] > b["y"]


def go_menu(st):
    st["mode"] = "menu"
    st["menu_i"] = 0
    st["score"] = 0
    st["mark"] = 0
    st["lives"] = 3


def down(key):
    return rl.IsKeyDown(key)


def hit(key):
    return rl.IsKeyPressed(key)


def want_left(st):
    if st["auto"]:
        return False
    return down(rl.KEY_LEFT) or down(rl.KEY_A)


def want_right(st):
    if st["auto"]:
        return True
    return down(rl.KEY_RIGHT) or down(rl.KEY_D)


def want_jump():
    return hit(rl.KEY_SPACE) or hit(rl.KEY_W) or hit(rl.KEY_UP)


def burst(st, x, y, color_name):
    i = 0
    while i < 8:
        ang = i * 0.8
        st["sparks"].append({
            "x": x,
            "y": y,
            "vx": math.cos(ang) * (1.4 + (i % 3)),
            "vy": math.sin(ang) * 1.6 - 2.2,
            "life": 16 + (i % 5),
            "kind": color_name,
        })
        i += 1


def lose_life(st, respawn):
    if st["mode"] != "play":
        return
    st["lives"] -= 1
    st["shake"] = 12
    if st["lives"] <= 0:
        st["mode"] = "dead"
        return
    st["invuln"] = 80
    hero = st["hero"]
    if respawn:
        spawn = current(st)["spawn"]
        hero["x"] = float(spawn[0])
        hero["y"] = float(spawn[1])
        hero["vx"] = 0
        hero["vy"] = 0
    else:
        hero["vx"] = -6 * hero["face"]
        hero["vy"] = -7


def finish_level(st):
    if st["mode"] != "play":
        return
    st["score"] += 250
    if st["level_i"] + 1 >= len(LEVELS):
        st["mode"] = "win"
    else:
        st["mode"] = "clear"


def update_menu(st):
    if hit(rl.KEY_UP) or hit(rl.KEY_W):
        st["menu_i"] = (st["menu_i"] - 1) % 3
    if hit(rl.KEY_DOWN) or hit(rl.KEY_S):
        st["menu_i"] = (st["menu_i"] + 1) % 3
    if hit(rl.KEY_ENTER) or hit(rl.KEY_SPACE):
        if st["menu_i"] == 0:
            st["score"] = 0
            st["mark"] = 0
            begin_level(st, 0, False)
        elif st["menu_i"] == 1:
            st["mode"] = "levels"
        else:
            st["quit"] = True


def update_levels(st):
    if hit(rl.KEY_LEFT) or hit(rl.KEY_A):
        st["card"] = (st["card"] - 1) % len(LEVELS)
    if hit(rl.KEY_RIGHT) or hit(rl.KEY_D):
        st["card"] = (st["card"] + 1) % len(LEVELS)
    if hit(rl.KEY_ESCAPE):
        go_menu(st)
    if hit(rl.KEY_ENTER) or hit(rl.KEY_SPACE):
        st["score"] = 0
        st["mark"] = 0
        begin_level(st, st["card"], False)


def update_pause(st):
    if hit(rl.KEY_UP) or hit(rl.KEY_W):
        st["pause_i"] = (st["pause_i"] - 1) % 2
    if hit(rl.KEY_DOWN) or hit(rl.KEY_S):
        st["pause_i"] = (st["pause_i"] + 1) % 2
    if hit(rl.KEY_ESCAPE) or hit(rl.KEY_P):
        st["mode"] = "play"
        return
    if hit(rl.KEY_ENTER) or hit(rl.KEY_SPACE):
        if st["pause_i"] == 0:
            st["mode"] = "play"
        else:
            go_menu(st)


def move_hero(st, plats):
    hero = st["hero"]
    if st["ground_i"] is not None and st["ground_i"] < len(st["plats"]) and st["ground_i"] < len(plats):
        old = st["plats"][st["ground_i"]]
        new = plats[st["ground_i"]]
        hero["x"] += new["x"] - old["x"]
        hero["y"] += new["y"] - old["y"]
    if want_left(st):
        hero["vx"] -= 0.75
    elif want_right(st):
        hero["vx"] += 0.75
    else:
        hero["vx"] *= 0.72
    if hero["vx"] > 5.1:
        hero["vx"] = 5.1
    if hero["vx"] < -5.1:
        hero["vx"] = -5.1
    if abs(hero["vx"]) < 0.08:
        hero["vx"] = 0
    if want_jump():
        hero["jump_buf"] = 8
    elif hero["jump_buf"] > 0:
        hero["jump_buf"] -= 1
    if hero["jump_buf"] > 0 and hero["coyote"] > 0:
        hero["vy"] = -11.2
        hero["jump_buf"] = 0
        hero["coyote"] = 0
        hero["ground"] = False
    if hero["vx"] > 0.25:
        hero["face"] = 1
    elif hero["vx"] < -0.25:
        hero["face"] = -1
    prev_x = hero["x"]
    hero["x"] += hero["vx"]
    for plat in plats:
        if overlaps(hero, plat):
            if prev_x + hero["w"] <= plat["x"] + 1:
                hero["x"] = plat["x"] - hero["w"]
                hero["vx"] = 0
            elif prev_x >= plat["x"] + plat["w"] - 1:
                hero["x"] = plat["x"] + plat["w"]
                hero["vx"] = 0
    width = current(st)["width"]
    if hero["x"] < 0:
        hero["x"] = 0
    if hero["x"] > width - hero["w"]:
        hero["x"] = width - hero["w"]
    was_ground = hero["ground"]
    prev_y = hero["y"]
    hero["prev_bottom"] = hero["y"] + hero["h"]
    hero["y"] += hero["vy"]
    hero["vy"] = min(14, hero["vy"] + 0.45)
    hero["ground"] = False
    ground_i = None
    i = 0
    while i < len(plats):
        plat = plats[i]
        feet = hero["y"] + hero["h"]
        prev_feet = prev_y + hero["h"]
        across = hero["x"] < plat["x"] + plat["w"] and hero["x"] + hero["w"] > plat["x"]
        if across and hero["vy"] >= 0 and prev_feet <= plat["y"] + 4 and feet >= plat["y"] - 0.01:
            hero["y"] = plat["y"] - hero["h"]
            hero["vy"] = 0
            hero["ground"] = True
            ground_i = i
        elif overlaps(hero, plat) and hero["vy"] < 0 and prev_y >= plat["y"] + plat["h"] - 4:
            hero["y"] = plat["y"] + plat["h"]
            hero["vy"] = 0
        i += 1
    st["ground_i"] = ground_i
    if hero["ground"]:
        hero["coyote"] = 8
        if not was_ground:
            hero["squash"] = 8
    elif hero["coyote"] > 0:
        hero["coyote"] -= 1
    if hero["squash"] > 0 and was_ground:
        hero["squash"] -= 1


def enemy_hit(st, enemy, box, top_pad):
    if not enemy["alive"] or st["mode"] != "play":
        return
    hero = st["hero"]
    if not overlaps(hero, box):
        return
    prev_bottom = hero["prev_bottom"]
    if hero["vy"] > 1 and prev_bottom <= box["y"] + top_pad:
        enemy["alive"] = False
        hero["vy"] = -8
        st["score"] += 100
        burst(st, box["x"] + box["w"] * 0.5, box["y"], "gold")
        return
    if st["invuln"] == 0:
        lose_life(st, False)


def update_play(st):
    if hit(rl.KEY_P) or hit(rl.KEY_ESCAPE):
        st["mode"] = "pause"
        st["pause_i"] = 0
        return
    plats = bake_platforms(st)
    move_hero(st, plats)
    hero = st["hero"]
    if st["invuln"] > 0:
        st["invuln"] -= 1
    if st["shake"] > 0:
        st["shake"] -= 1
    for moth in st["moths"]:
        if not moth["alive"]:
            continue
        moth["x"] += moth["dir"] * moth["speed"]
        if moth["x"] < moth["minx"] or moth["x"] > moth["maxx"]:
            moth["dir"] *= -1
        bob = math.sin(st["time"] * 0.08 + moth["phase"]) * 14
        moth["draw_y"] = moth["y"] + bob
        box = {"x": moth["x"], "y": moth["draw_y"], "w": moth["w"], "h": moth["h"]}
        enemy_hit(st, moth, box, 12)
    for bug in st["beetles"]:
        if not bug["alive"]:
            continue
        bug["x"] += bug["dir"] * bug["speed"]
        if bug["x"] < bug["minx"] or bug["x"] > bug["maxx"]:
            bug["dir"] *= -1
            bug["x"] += bug["dir"] * bug["speed"]
        enemy_hit(st, bug, bug, 14)
    taken = 0
    total = 0
    for wisp in st["wisps"]:
        total += 1
        if wisp["got"]:
            taken += 1
            continue
        bob = math.sin(st["time"] * 0.09 + wisp["phase"]) * 8
        box = {"x": wisp["x"] - 16, "y": wisp["y"] + bob - 16, "w": 32, "h": 32}
        if overlaps(hero, box):
            wisp["got"] = True
            taken += 1
            st["score"] += 50
            burst(st, wisp["x"], wisp["y"], "blue")
        wisp["draw_y"] = wisp["y"] + bob
    portal = current(st)["portal"]
    door = {"x": portal[0], "y": portal[1], "w": portal[2], "h": portal[3]}
    if taken == total and overlaps(hero, door):
        finish_level(st)
    if hero["y"] > 780:
        lose_life(st, True)
    if st["banner"] > 0:
        st["banner"] -= 1
    kept = []
    for spark in st["sparks"]:
        spark["life"] -= 1
        if spark["life"] <= 0:
            continue
        spark["x"] += spark["vx"]
        spark["y"] += spark["vy"]
        spark["vy"] += 0.12
        kept.append(spark)
    st["sparks"] = kept
    st["plats"] = plats


def update_clear(st):
    if hit(rl.KEY_ENTER) or hit(rl.KEY_SPACE):
        begin_level(st, st["level_i"] + 1, False)


def update_dead(st):
    if hit(rl.KEY_ENTER) or hit(rl.KEY_SPACE):
        begin_level(st, st["level_i"], True)
    if hit(rl.KEY_ESCAPE):
        go_menu(st)


def update_win(st):
    if hit(rl.KEY_ENTER) or hit(rl.KEY_SPACE) or hit(rl.KEY_ESCAPE):
        go_menu(st)


def update(st):
    st["time"] += 1
    mode = st["mode"]
    if mode == "menu":
        update_menu(st)
    elif mode == "levels":
        update_levels(st)
    elif mode == "play":
        update_play(st)
    elif mode == "pause":
        update_pause(st)
    elif mode == "clear":
        update_clear(st)
    elif mode == "dead":
        update_dead(st)
    elif mode == "win":
        update_win(st)


def text(message, x, y, size, color):
    rl.DrawText(message.encode(), int(x), int(y), int(size), color)


def text_center(message, cx, y, size, color):
    width = rl.MeasureText(message.encode(), int(size))
    text(message, cx - width / 2, y, size, color)


def panel(x, y, w, h):
    rl.DrawRectangle(int(x) - 2, int(y) - 2, int(w) + 4, int(h) + 4, GOLD)
    rl.DrawRectangle(int(x), int(y), int(w), int(h), PANEL)


def camera_of(st):
    if st["mode"] == "menu" or st["mode"] == "levels":
        return 0
    hero = st["hero"]
    width = current(st)["width"]
    cam = hero["x"] + hero["face"] * 70 - 400
    if cam < 0:
        cam = 0
    if cam > width - SCREEN_W:
        cam = width - SCREEN_W
    if st["shake"] > 0:
        cam += math.sin(st["time"] * 1.8) * st["shake"]
    return cam


def draw_sky(st, bank, cam):
    rl.DrawRectangle(0, 0, SCREEN_W, SCREEN_H, SKY)
    rl.DrawRectangle(0, 250, SCREEN_W, SCREEN_H - 250, DUSK)
    for star in st["stars"]:
        twinkle = 150 + int(80 * (0.5 + 0.5 * math.sin(st["time"] * 0.07 + star["p"])))
        rl.DrawCircle(int(star["x"]), int(star["y"]), 2, (255, 244, 210, twinkle))
    moon_y = 28 + math.sin(st["time"] * 0.02) * 8
    svgload.draw_sprite(bank["moon"], 760, moon_y, 130, 130, False, rl.WHITE)
    drift = st["time"] * 0.35
    clouds = ((40, 70, 210, 94), (380, 110, 250, 110), (680, 48, 180, 80))
    i = 0
    while i < 3:
        cx, cy, cw, ch = clouds[i]
        x = (cx + drift * (0.6 + i * 0.25) - cam * 0.15) % (SCREEN_W + cw) - cw
        svgload.draw_sprite(bank["cloud"], x, cy, cw, ch, False, rl.WHITE)
        i += 1
    hill_w = 560
    start = int(-((cam * 0.38) % hill_w)) - hill_w
    x = start
    while x < SCREEN_W:
        svgload.draw_sprite(bank["hill"], x, 292, hill_w, 210, False, rl.WHITE)
        x += hill_w - 2


def draw_platforms(st, bank, cam):
    plats = st["plats"]
    if st["mode"] != "play" and st["mode"] != "pause" and st["mode"] != "clear" and st["mode"] != "dead" and st["mode"] != "win":
        plats = bake_platforms(st)
    for plat in plats:
        tint = rl.WHITE if plat["kind"] == "ground" else (176, 166, 204, 255)
        tile = 64.0
        x = plat["x"]
        dest_h = 64 if plat["kind"] == "ground" else plat["h"]
        while x < plat["x"] + plat["w"] - 0.1:
            remain = plat["x"] + plat["w"] - x
            tw = tile if remain > tile else remain
            src_w = rl.tex_w(bank["ground"]) * (tw / tile)
            src_h = rl.tex_h(bank["ground"]) if plat["kind"] == "ground" else rl.tex_h(bank["ground"]) * 0.42
            rl.sprite_src(bank["ground"], 0, 0, src_w, src_h, x - cam, plat["y"], tw, dest_h, tint)
            x += tw


def hero_frame(st, bank):
    hero = st["hero"]
    if not hero["ground"]:
        frames = bank["hero_jump"]
        step = 1
    elif abs(hero["vx"]) > 0.4:
        frames = bank["hero_run"]
        step = 12
    else:
        frames = bank["hero_idle"]
        step = 24
    return frames[(st["time"] // step) % len(frames)]


def draw_hero(st, bank, cam):
    hero = st["hero"]
    if st["invuln"] > 0 and (st["time"] // 4) % 2 == 0:
        return
    sx = 1.0
    sy = 1.0
    if hero["squash"] > 0:
        sx = 1.14
        sy = 0.78
    dw = 120 * sx
    dh = 150 * sy
    feet = hero["y"] + hero["h"]
    center = hero["x"] + hero["w"] / 2
    x = center - dw / 2 - cam
    y = feet - dh
    glow = 56 + math.sin(st["time"] * 0.05) * 3
    gx = center + hero["face"] * 34 - cam - glow / 2
    gy = feet - 58 - glow / 2
    pulse = bank["wisp"][(st["time"] // 10) % 2]
    svgload.draw_sprite(pulse, gx, gy, glow, glow, False, (255, 214, 120, 170))
    svgload.draw_sprite(hero_frame(st, bank), x, y, dw, dh, hero["face"] < 0, rl.WHITE)


def draw_actors(st, bank, cam):
    for wisp in st["wisps"]:
        if wisp["got"]:
            continue
        y = wisp.get("draw_y", wisp["y"])
        frame = bank["wisp"][(st["time"] // 9) % 2]
        svgload.draw_sprite(frame, wisp["x"] - 18 - cam, y - 18, 36, 36, False, rl.WHITE)
    portal = current(st)["portal"]
    open_door = True
    for wisp in st["wisps"]:
        if not wisp["got"]:
            open_door = False
    tint = rl.WHITE if open_door else LOCKED
    frame = bank["portal"][(st["time"] // 8) % 4]
    svgload.draw_sprite(frame, portal[0] - cam, portal[1], portal[2], portal[3], False, tint)
    for moth in st["moths"]:
        if not moth["alive"]:
            continue
        y = moth.get("draw_y", moth["y"])
        frame = bank["moth"][(st["time"] // 8) % 2]
        svgload.draw_sprite(frame, moth["x"] - cam, y, moth["w"] + 16, moth["h"] + 10, moth["dir"] < 0, rl.WHITE)
    for bug in st["beetles"]:
        if not bug["alive"]:
            continue
        frame = bank["beetle"][(st["time"] // 10) % 2]
        svgload.draw_sprite(frame, bug["x"] - cam, bug["y"] - 8, bug["w"] + 10, bug["h"] + 12, bug["dir"] < 0, rl.WHITE)
    for spark in st["sparks"]:
        color = GOLD if spark["kind"] == "gold" else (170, 230, 255, 255)
        rl.DrawCircle(int(spark["x"] - cam), int(spark["y"]), max(1, spark["life"] // 4), color)


def draw_hud(st, bank):
    text("score " + str(st["score"]), 24, 18, 22, INK)
    i = 0
    while i < 3:
        frame = bank["heart_full"] if i < st["lives"] else bank["heart_empty"]
        svgload.draw_sprite(frame, SCREEN_W - 150 + i * 42, 14, 36, 36, False, rl.WHITE)
        i += 1
    if st["banner"] > 0 and st["mode"] == "play":
        src = current(st)
        text(src["name"], 24, 48, 28, GOLD)
        text("collect every wisp, then step into the gate", 24, 84, 18, DIM)
        text("A D move    W or space jump    P pause", 24, SCREEN_H - 32, 18, DIM)


def draw_world(st, bank):
    cam = camera_of(st)
    draw_sky(st, bank, cam)
    draw_platforms(st, bank, cam)
    if st["mode"] != "menu" and st["mode"] != "levels":
        draw_actors(st, bank, cam)
        draw_hero(st, bank, cam)
    draw_hud(st, bank)


def draw_menu(st, bank):
    draw_sky(st, bank, st["time"] * 0.4)
    svgload.draw_sprite(bank["hill"], -40, 330, 640, 230, False, rl.WHITE)
    svgload.draw_sprite(bank["hill"], 420, 350, 580, 210, False, rl.WHITE)
    moth = bank["moth"][(st["time"] // 8) % 2]
    mx = (st["time"] * 2.2) % (SCREEN_W + 160) - 80
    my = 150 + math.sin(st["time"] * 0.05) * 24
    svgload.draw_sprite(moth, mx, my, 90, 60, False, rl.WHITE)
    frame = bank["hero_idle"][(st["time"] // 24) % 2]
    svgload.draw_sprite(frame, 70, 168, 280, 350, False, rl.WHITE)
    wisp = bank["wisp"][(st["time"] // 9) % 2]
    svgload.draw_sprite(wisp, 250, 250 + math.sin(st["time"] * 0.1) * 10, 48, 48, False, rl.WHITE)
    text("LUMEN", 500, 78, 72, GOLD)
    text("a lantern kept for the dark", 508, 160, 22, DIM)
    labels = ("Start", "Levels", "Quit")
    i = 0
    while i < 3:
        y = 240 + i * 64
        color = GOLD if i == st["menu_i"] else INK
        if i == st["menu_i"]:
            svgload.draw_sprite(wisp, 500, y - 6, 32, 32, False, rl.WHITE)
        text(labels[i], 548, y, 36, color)
        i += 1
    text("up and down, enter to choose", 508, 470, 18, DIM)


def draw_levels(st, bank):
    draw_sky(st, bank, 0)
    text("Choose a path", 48, 36, 36, GOLD)
    i = 0
    while i < len(LEVELS):
        src = LEVELS[i]
        x = 48 + i * 300
        y = 110
        if i == st["card"]:
            panel(x, y, 270, 340)
        else:
            rl.DrawRectangle(x, y, 270, 340, (18, 20, 40, 210))
        text(str(i + 1), x + 18, y + 16, 22, DIM)
        text(src["name"], x + 18, y + 52, 26, GOLD if i == st["card"] else INK)
        text(src["blurb"], x + 18, y + 110, 18, DIM)
        text(src["detail"], x + 18, y + 136, 18, DIM)
        text(str(len(src["wisps"])) + " wisps", x + 18, y + 190, 18, INK)
        text(str(len(src["moths"])) + " moths", x + 18, y + 216, 18, INK)
        text(str(len(src["beetles"])) + " beetles", x + 18, y + 242, 18, INK)
        hero = bank["hero_idle"][(st["time"] // 24) % 2]
        svgload.draw_sprite(hero, x + 150, y + 180, 100, 130, False, rl.WHITE)
        i += 1
    text("left and right, enter to play, esc back", 48, 490, 18, DIM)


def draw_overlay(st):
    if st["mode"] == "pause":
        panel(280, 150, 400, 230)
        text("Paused", 330, 176, 40, GOLD)
        labels = ("Resume", "Menu")
        i = 0
        while i < 2:
            color = GOLD if i == st["pause_i"] else INK
            text(labels[i], 360, 250 + i * 48, 28, color)
            i += 1
    elif st["mode"] == "clear":
        panel(230, 160, 500, 200)
        text("The gate opens", 300, 190, 36, GOLD)
        text("score " + str(st["score"]), 400, 246, 22, INK)
        text("enter for the next path", 330, 292, 20, DIM)
    elif st["mode"] == "dead":
        panel(230, 160, 500, 210)
        text("The light went out", 280, 188, 34, GOLD)
        text("enter to try again", 350, 252, 22, INK)
        text("esc returns to the menu", 330, 292, 20, DIM)
    elif st["mode"] == "win":
        panel(200, 140, 560, 240)
        text("The lantern is home", 250, 172, 36, GOLD)
        text("score " + str(st["score"]), 400, 236, 28, INK)
        text("enter returns to the menu", 310, 300, 20, DIM)


def draw(st, bank):
    rl.BeginDrawing()
    if st["mode"] == "menu":
        draw_menu(st, bank)
    elif st["mode"] == "levels":
        draw_levels(st, bank)
    else:
        draw_world(st, bank)
        draw_overlay(st)
    rl.EndDrawing()


def load_bank():
    return {
        "hero_idle": svgload.load_clip("hero.svg", ["idle0", "idle1"]),
        "hero_run": svgload.load_clip("hero.svg", ["run0", "run1", "run2", "run3"]),
        "hero_jump": svgload.load_clip("hero.svg", ["jump"]),
        "moth": svgload.load_clip("moth.svg", ["flap0", "flap1"]),
        "beetle": svgload.load_clip("beetle.svg", ["walk0", "walk1"]),
        "wisp": svgload.load_clip("wisp.svg", ["pulse0", "pulse1"]),
        "portal": svgload.load_clip("portal.svg", ["spin0", "spin1", "spin2", "spin3"]),
        "heart_full": svgload.load_clip("heart.svg", ["full"])[0],
        "heart_empty": svgload.load_clip("heart.svg", ["empty"])[0],
        "ground": svgload.load_clip("ground.svg", [None])[0],
        "hill": svgload.load_clip("hill.svg", [None])[0],
        "moon": svgload.load_clip("moon.svg", [None])[0],
        "cloud": svgload.load_clip("cloud.svg", [None])[0],
    }


def unload(bank):
    lists = ("hero_idle", "hero_run", "hero_jump", "moth", "beetle", "wisp", "portal")
    singles = ("heart_full", "heart_empty", "ground", "hill", "moon", "cloud")
    for name in lists:
        for texture in bank[name]:
            rl.UnloadTexture(texture)
    for name in singles:
        rl.UnloadTexture(bank[name])


def play(frames):
    rl.InitWindow(SCREEN_W, SCREEN_H, b"Lumen")
    rl.SetExitKey(rl.KEY_NULL)
    rl.SetTargetFPS(60)
    bank = None
    summary = "closed"
    try:
        bank = load_bank()
        st = new_state()
        if frames > 0:
            begin_level(st, 0, False)
            st["auto"] = True
        i = 0
        while not rl.WindowShouldClose() and not st["quit"]:
            if frames > 0 and i >= frames:
                break
            update(st)
            draw(st, bank)
            i += 1
        summary = "mode %s level %s x %d score %d" % (
            st["mode"],
            st["level_i"] + 1,
            int(st["hero"]["x"]),
            st["score"],
        )
    finally:
        if bank is not None:
            unload(bank)
        rl.CloseWindow()
    return summary
