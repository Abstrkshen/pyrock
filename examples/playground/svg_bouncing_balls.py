# mode: native-only
# Several SVGs printed one after another play as an animation:
# each <svg> is one frame. The physics compiles to native Rust.
W, H, R = 480, 300, 12

class Ball:
    def __init__(self, x, y, vx, vy, color):
        self.x = x
        self.y = y
        self.vx = vx
        self.vy = vy
        self.color = color

    def step(self, dt):
        self.vy += 900.0 * dt
        self.x += self.vx * dt
        self.y += self.vy * dt
        if self.x < R or self.x > W - R:
            self.vx = -self.vx
            self.x = min(max(self.x, R), W - R)
        if self.y > H - R:
            self.vy = -self.vy * 0.88
            self.y = H - R

colors = ["#0f6e57", "#c99a1e", "#2c55a0", "#a0521a", "#56626a"]
balls = [Ball(40.0 + i * 85, 40.0 + i * 20, 120.0 + i * 35, 0.0, colors[i % 5]) for i in range(5)]

for frame in range(150):
    for _ in range(4):
        for b in balls:
            b.step(1 / 240)
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}">',
             f'<rect width="{W}" height="{H}" fill="#f3f4f0"/>']
    for b in balls:
        parts.append(f'<circle cx="{b.x:.1f}" cy="{b.y:.1f}" r="{R}" fill="{b.color}"/>')
    parts.append("</svg>")
    print("".join(parts))
