# The loop of game_state_dicts.py with the state in classes, 200k frames.
class Rect:
    def __init__(self, x, y, w, h):
        self.x = x
        self.y = y
        self.w = w
        self.h = h


class Platform(Rect):
    def __init__(self, x, y, w, h, kind):
        super().__init__(x, y, w, h)
        self.kind = kind


class Hero(Rect):
    def __init__(self, x, y):
        super().__init__(x, y, 20.0, 30.0)
        self.vx = 2.5
        self.vy = 0.0
        self.ground = False


class State:
    def __init__(self):
        self.mode = "play"
        self.time = 0
        self.score = 0
        self.plats = []
        for i in range(20):
            self.plats.append(Platform(i * 50.0, 400.0 - (i % 5) * 30, 48.0, 12.0, "ground" if i % 3 else "ledge"))
        self.hero = Hero(80.0, 100.0)


def overlaps(a, b):
    return a.x < b.x + b.w and a.x + a.w > b.x and a.y < b.y + b.h and a.y + a.h > b.y


def update(st):
    hero = st.hero
    hero.vy += 0.4
    hero.x += hero.vx
    hero.y += hero.vy
    hero.ground = False
    for p in st.plats:
        if overlaps(hero, p) and hero.vy > 0:
            hero.y = p.y - hero.h
            hero.vy = -9.0 if p.kind == "ledge" else 0.0
            hero.ground = True
            st.score += 1
    if hero.x > 1000:
        hero.x = 0.0
    if hero.y > 600:
        hero.y = 0.0
        hero.vy = 0.0
    st.time += 1


st = State()
for _ in range(200000):
    update(st)
print(st.score, st.time, round(st.hero.x, 3), round(st.hero.y, 3))
