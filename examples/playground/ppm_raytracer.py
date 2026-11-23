# mode: native-only
# A small ray tracer: three spheres on a checkered floor, lit by one light,
# with shadows and a reflection bounce. Prints one P3 (plain PPM) image.
# pyrock --release: 30 ms; CPython 3.12: 321 ms (whole run, this machine)
import math

W, H = 200, 140


class Vec:
    def __init__(self, x, y, z):
        self.x = x
        self.y = y
        self.z = z

    def add(self, o):
        return Vec(self.x + o.x, self.y + o.y, self.z + o.z)

    def sub(self, o):
        return Vec(self.x - o.x, self.y - o.y, self.z - o.z)

    def mul(self, k):
        return Vec(self.x * k, self.y * k, self.z * k)

    def dot(self, o):
        return self.x * o.x + self.y * o.y + self.z * o.z

    def norm(self):
        return self.mul(1.0 / math.sqrt(self.dot(self)))


class Sphere:
    def __init__(self, center, radius, color, shine):
        self.center = center
        self.radius = radius
        self.color = color
        self.shine = shine

    def hit(self, origin, d):
        oc = origin.sub(self.center)
        b = oc.dot(d)
        c = oc.dot(oc) - self.radius * self.radius
        disc = b * b - c
        if disc < 0.0:
            return -1.0
        s = math.sqrt(disc)
        t = -b - s
        if t > 0.001:
            return t
        t = -b + s
        if t > 0.001:
            return t
        return -1.0


spheres = [
    Sphere(Vec(-1.1, 0.0, 4.2), 1.0, Vec(15.0, 110.0, 87.0), 0.25),
    Sphere(Vec(1.0, -0.35, 3.4), 0.65, Vec(201.0, 154.0, 30.0), 0.5),
    Sphere(Vec(0.35, 0.55, 6.0), 1.55, Vec(44.0, 85.0, 160.0), 0.35),
]
light = Vec(-3.0, 5.0, 0.5)
FLOOR_Y = -1.0


def floor_color(p):
    if (math.floor(p.x * 1.2) + math.floor(p.z * 1.2)) % 2 == 0:
        return Vec(236.0, 238.0, 233.0)
    return Vec(86.0, 98.0, 106.0)


def shadowed(p):
    to_light = light.sub(p)
    dist = math.sqrt(to_light.dot(to_light))
    d = to_light.norm()
    for s in spheres:
        t = s.hit(p, d)
        if t > 0.0 and t < dist:
            return True
    return False


def trace(origin, d, depth):
    best_t = 1e9
    best = -1
    for i in range(len(spheres)):
        t = spheres[i].hit(origin, d)
        if t > 0.0 and t < best_t:
            best_t = t
            best = i
    floor_t = -1.0
    if d.y < -0.0001:
        floor_t = (FLOOR_Y - origin.y) / d.y
    if best < 0 and floor_t < 0.0:
        sky = 0.5 + 0.5 * d.y
        return Vec(236.0 - 60.0 * sky, 238.0 - 40.0 * sky, 233.0 - 10.0 * sky)
    if best < 0 or (floor_t > 0.0 and floor_t < best_t):
        p = origin.add(d.mul(floor_t))
        normal = Vec(0.0, 1.0, 0.0)
        base = floor_color(p)
        shine = 0.15
    else:
        s = spheres[best]
        p = origin.add(d.mul(best_t))
        normal = p.sub(s.center).norm()
        base = s.color
        shine = s.shine
    l = light.sub(p).norm()
    diffuse = max(0.0, normal.dot(l))
    if shadowed(p):
        diffuse = diffuse * 0.25
    color = base.mul(0.25 + 0.75 * diffuse)
    half = l.sub(d).norm()
    spec = max(0.0, normal.dot(half)) ** 40
    color = color.add(Vec(255.0, 255.0, 255.0).mul(spec * shine * (0.0 if shadowed(p) else 1.0)))
    if depth > 0 and shine > 0.0:
        r = d.sub(normal.mul(2.0 * d.dot(normal)))
        bounce = trace(p, r.norm(), depth - 1)
        color = color.mul(1.0 - shine).add(bounce.mul(shine))
    return color


def clamp(v):
    return min(255, max(0, int(v)))


eye = Vec(0.0, 0.3, -1.0)
print("P3")
print(W, H)
print(255)
for y in range(H):
    row = []
    for x in range(W):
        d = Vec((x - W / 2) / H, (H / 2 - y) / H, 1.0).norm()
        c = trace(eye, d, 2)
        row.append(f"{clamp(c.x)} {clamp(c.y)} {clamp(c.z)}")
    print(" ".join(row))
