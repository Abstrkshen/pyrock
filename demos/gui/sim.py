"""Ball physics: walls, gravity and pairwise elastic collisions.
Compiled natively; the window code in main.py only draws the result."""
from dataclasses import dataclass


@dataclass
class Ball:
    x: float
    y: float
    vx: float
    vy: float
    r: float
    hue: int


class World:
    def __init__(self, width, height, count, seed):
        self.width = width
        self.height = height
        self.balls = []
        self.collisions = 0
        x = seed
        for i in range(count):
            x = (x * 1103515245 + 12345) % 2147483648
            r = 4.0 + x % 9
            x = (x * 1103515245 + 12345) % 2147483648
            px = r + (x % 10000) / 10000.0 * (width - 2 * r)
            x = (x * 1103515245 + 12345) % 2147483648
            py = r + (x % 10000) / 10000.0 * (height / 2 - 2 * r)
            x = (x * 1103515245 + 12345) % 2147483648
            vx = (x % 2001 - 1000) / 250.0
            self.balls.append(Ball(px, py, vx, 0.0, r, i % 6))

    def step(self, dt):
        for b in self.balls:
            b.vy += 0.15 * dt
            b.x += b.vx * dt
            b.y += b.vy * dt
            if b.x < b.r:
                b.x = b.r
                b.vx = -b.vx * 0.9
            elif b.x > self.width - b.r:
                b.x = self.width - b.r
                b.vx = -b.vx * 0.9
            if b.y > self.height - b.r:
                b.y = self.height - b.r
                b.vy = -b.vy * 0.85
        balls = self.balls
        n = len(balls)
        for i in range(n):
            a = balls[i]
            for j in range(i + 1, n):
                b = balls[j]
                dx = b.x - a.x
                dy = b.y - a.y
                dist2 = dx * dx + dy * dy
                rr = a.r + b.r
                if dist2 < rr * rr and dist2 > 1e-9:
                    dist = dist2 ** 0.5
                    nx = dx / dist
                    ny = dy / dist
                    rel = (a.vx - b.vx) * nx + (a.vy - b.vy) * ny
                    if rel > 0:
                        ma = a.r * a.r
                        mb = b.r * b.r
                        k = 2 * rel / (ma + mb)
                        a.vx -= k * mb * nx
                        a.vy -= k * mb * ny
                        b.vx += k * ma * nx
                        b.vy += k * ma * ny
                        self.collisions += 1
                    push = (rr - dist) / 2
                    a.x -= nx * push
                    a.y -= ny * push
                    b.x += nx * push
                    b.y += ny * push

    def energy(self):
        e = 0.0
        for b in self.balls:
            e += 0.5 * b.r * b.r * (b.vx * b.vx + b.vy * b.vy) + b.r * b.r * 0.15 * (self.height - b.y)
        return e
