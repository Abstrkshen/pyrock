# mode: native-only
# Two double pendulums that start one thousandth of a radian apart.
# For a few seconds they move together, then chaos pulls them apart.
# Integrated with a fourth-order Runge-Kutta method; prints one SVG per frame.
# pyrock --release: 6 ms; CPython 3.12: 28 ms (whole run, this machine)
import math

W, H = 520, 400
G, L1, L2, M1, M2 = 9.81, 1.0, 1.0, 1.0, 1.0
SCALE = 90.0
OX, OY = W / 2, 150.0
DT, SUBSTEPS, FRAMES = 0.004, 18, 180
TRAIL = 36


def accel(t1, w1, t2, w2):
    d = t2 - t1
    c, s = math.cos(d), math.sin(d)
    den1 = (M1 + M2) * L1 - M2 * L1 * c * c
    a1 = (M2 * L1 * w1 * w1 * s * c + M2 * G * math.sin(t2) * c
          + M2 * L2 * w2 * w2 * s - (M1 + M2) * G * math.sin(t1)) / den1
    den2 = (L2 / L1) * den1
    a2 = (-M2 * L2 * w2 * w2 * s * c
          + (M1 + M2) * (G * math.sin(t1) * c - L1 * w1 * w1 * s - G * math.sin(t2))) / den2
    return a1, a2


class Pendulum:
    def __init__(self, t1, t2, color):
        self.t1 = t1
        self.w1 = 0.0
        self.t2 = t2
        self.w2 = 0.0
        self.color = color
        self.trail = []

    def step(self, h):
        # Runge-Kutta-Nystrom, fourth order: angles and angular velocities together.
        t1, w1, t2, w2 = self.t1, self.w1, self.t2, self.w2
        a1, b1 = accel(t1, w1, t2, w2)
        a2, b2 = accel(t1 + h / 2 * w1 + h * h / 8 * a1, w1 + h / 2 * a1,
                       t2 + h / 2 * w2 + h * h / 8 * b1, w2 + h / 2 * b1)
        a3, b3 = accel(t1 + h / 2 * w1 + h * h / 8 * a1, w1 + h / 2 * a2,
                       t2 + h / 2 * w2 + h * h / 8 * b1, w2 + h / 2 * b2)
        a4, b4 = accel(t1 + h * w1 + h * h / 2 * a3, w1 + h * a3,
                       t2 + h * w2 + h * h / 2 * b3, w2 + h * b3)
        self.t1 = t1 + h * w1 + h * h / 6 * (a1 + a2 + a3)
        self.w1 = w1 + h / 6 * (a1 + 2 * a2 + 2 * a3 + a4)
        self.t2 = t2 + h * w2 + h * h / 6 * (b1 + b2 + b3)
        self.w2 = w2 + h / 6 * (b1 + 2 * b2 + 2 * b3 + b4)

    def joints(self):
        x1 = OX + SCALE * L1 * math.sin(self.t1)
        y1 = OY + SCALE * L1 * math.cos(self.t1)
        x2 = x1 + SCALE * L2 * math.sin(self.t2)
        y2 = y1 + SCALE * L2 * math.cos(self.t2)
        return x1, y1, x2, y2

    def draw(self):
        x1, y1, x2, y2 = self.joints()
        self.trail.append(f"{x2:.1f},{y2:.1f}")
        if len(self.trail) > TRAIL:
            self.trail.pop(0)
        out = []
        # The trail fades: four pieces, older ones more transparent.
        piece = TRAIL // 4
        for k in range(4):
            pts = self.trail[k * piece:(k + 1) * piece + 1]
            if len(pts) > 1:
                out.append(f'<polyline points="{" ".join(pts)}" fill="none" stroke="{self.color}" '
                           f'stroke-width="2" opacity="{0.15 + 0.2 * k:.2f}"/>')
        out.append(f'<line x1="{OX:.1f}" y1="{OY:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" stroke="#56626A" stroke-width="3"/>')
        out.append(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke="#56626A" stroke-width="3"/>')
        out.append(f'<circle cx="{x1:.1f}" cy="{y1:.1f}" r="7" fill="{self.color}"/>')
        out.append(f'<circle cx="{x2:.1f}" cy="{y2:.1f}" r="9" fill="{self.color}"/>')
        return "".join(out)


a = Pendulum(2.4, 2.1, "#0F6E57")
b = Pendulum(2.401, 2.1, "#C99A1E")
for frame in range(FRAMES):
    for _ in range(SUBSTEPS):
        a.step(DT)
        b.step(DT)
    t = (frame + 1) * SUBSTEPS * DT
    gap = abs(a.t2 - b.t2) % (2 * math.pi)
    if gap > math.pi:
        gap = 2 * math.pi - gap
    print(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}">'
          f'<rect width="{W}" height="{H}" fill="#F3F4F0"/>'
          + a.draw() + b.draw()
          + f'<circle cx="{OX:.1f}" cy="{OY:.1f}" r="4" fill="#1C2326"/>'
          f'<text x="16" y="28" font-family="sans-serif" font-size="14" fill="#1C2326">t = {t:.2f} s</text>'
          f'<text x="16" y="48" font-family="sans-serif" font-size="13" fill="#56626A">'
          f'lower arms differ by {gap:.4f} rad</text></svg>')
