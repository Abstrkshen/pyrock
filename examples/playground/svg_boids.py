# mode: native-only
# Flocking: each bird steers away from close neighbours, matches their heading
# and drifts towards their centre. No bird knows about the flock as a whole.
# Prints one SVG per frame; the playground plays them as an animation.
# pyrock --release: 19 ms; CPython 3.12: 42 ms (whole run, this machine)
import math

W, H = 520, 340
N = 40
VIEW, CLOSE = 60.0, 18.0
MAX_SPEED = 3.2
MIN_SPEED = 1.8

seed = 7


def rand():
    global seed
    seed = (seed * 1103515245 + 12345) % 2147483648
    return (seed >> 8) / 8388608.0  # in [0, 1), from the high bits


class Boid:
    def __init__(self, x, y, vx, vy):
        self.x = x
        self.y = y
        self.vx = vx
        self.vy = vy


flock = [Boid(rand() * W, rand() * H, rand() * 4 - 2, rand() * 4 - 2) for _ in range(N)]


def step(flock):
    moves = []
    for b in flock:
        sx = sy = ax = ay = cx = cy = 0.0
        seen = 0
        for o in flock:
            if o is b:
                continue
            dx, dy = o.x - b.x, o.y - b.y
            d2 = dx * dx + dy * dy
            if d2 < VIEW * VIEW:
                seen += 1
                ax += o.vx
                ay += o.vy
                cx += o.x
                cy += o.y
                if d2 < CLOSE * CLOSE:
                    sx -= dx
                    sy -= dy
        vx, vy = b.vx, b.vy
        if seen > 0:
            vx += (ax / seen - vx) * 0.12 + (cx / seen - b.x) * 0.003 + sx * 0.06
            vy += (ay / seen - vy) * 0.12 + (cy / seen - b.y) * 0.003 + sy * 0.06
        # Turn back from the edges.
        margin = 40.0
        if b.x < margin:
            vx += 0.3
        if b.x > W - margin:
            vx -= 0.3
        if b.y < margin:
            vy += 0.3
        if b.y > H - margin:
            vy -= 0.3
        speed = math.sqrt(vx * vx + vy * vy)
        if speed > MAX_SPEED:
            vx, vy = vx / speed * MAX_SPEED, vy / speed * MAX_SPEED
        elif speed < MIN_SPEED and speed > 0.0:
            vx, vy = vx / speed * MIN_SPEED, vy / speed * MIN_SPEED
        moves.append((vx, vy))
    for i in range(len(flock)):
        b = flock[i]
        b.vx, b.vy = moves[i]
        b.x += b.vx
        b.y += b.vy


def bird(b, color):
    speed = math.sqrt(b.vx * b.vx + b.vy * b.vy) or 1.0
    ux, uy = b.vx / speed, b.vy / speed
    px, py = -uy, ux
    tip = (b.x + ux * 9, b.y + uy * 9)
    left = (b.x - ux * 5 + px * 4.5, b.y - uy * 5 + py * 4.5)
    right = (b.x - ux * 5 - px * 4.5, b.y - uy * 5 - py * 4.5)
    pts = " ".join([f"{x:.1f},{y:.1f}" for x, y in (tip, left, right)])
    return f'<polygon points="{pts}" fill="{color}"/>'


COLORS = ["#0F6E57", "#2C55A0", "#C99A1E", "#A0521A"]
for frame in range(150):
    step(flock)
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}">',
             f'<rect width="{W}" height="{H}" fill="#F3F4F0"/>']
    for i in range(len(flock)):
        parts.append(bird(flock[i], COLORS[i % len(COLORS)]))
    parts.append("</svg>")
    print("".join(parts))
