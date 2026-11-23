# mode: native-only
# A fractal plant from an L-system: two rewriting rules applied six times,
# then drawn by a turtle that saves and restores its position on a stack.
# Prints one SVG; branches get thinner and greener the deeper they are.
# pyrock --release: 12 ms; CPython 3.12: 22 ms (whole run, this machine)
import math

RULES = {"X": "F+[[X]-X]-F[-FX]+X", "F": "FF"}
ANGLE = 25.0
STEP = 3.4
H = 560  # picture height; the width follows the plant's shape

s = "X"
for _ in range(6):
    s = "".join([RULES.get(c, c) for c in s])

# Walk the string. Segments are grouped by branch depth: one path per depth.
x, y, heading = 0.0, 0.0, -75.0
lo_x = hi_x = lo_y = hi_y = 0.0
stack = []
paths = {}
leaves = []
depth = 0
for c in s:
    if c == "F":
        rad = math.radians(heading)
        nx, ny = x + STEP * math.cos(rad), y + STEP * math.sin(rad)
        if depth not in paths:
            paths[depth] = []
        paths[depth].append(f"M{x:.1f} {y:.1f}L{nx:.1f} {ny:.1f}")
        x, y = nx, ny
        lo_x, hi_x = min(lo_x, x), max(hi_x, x)
        lo_y, hi_y = min(lo_y, y), max(hi_y, y)
    elif c == "+":
        heading += ANGLE
    elif c == "-":
        heading -= ANGLE
    elif c == "[":
        stack.append((x, y, heading, depth))
        depth += 1
    elif c == "]":
        if depth >= 4:
            leaves.append((x, y))
        x, y, heading, depth = stack.pop()


def mix(a, b, t):
    return int(a + (b - a) * t)


# Fit the drawing: the viewBox is its measured bounds plus a margin.
pad = 16.0
vx, vy = lo_x - pad, lo_y - pad
vw, vh = hi_x - lo_x + 2 * pad, hi_y - lo_y + 2 * pad
W = int(H * vw / vh)
deepest = max(paths.keys())
out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="{vx:.1f} {vy:.1f} {vw:.1f} {vh:.1f}">',
       f'<rect x="{vx:.1f}" y="{vy:.1f}" width="{vw:.1f}" height="{vh:.1f}" fill="#ECEEE9"/>',
       f'<ellipse cx="0" cy="2" rx="{vw * 0.22:.1f}" ry="6" fill="#56626A" opacity="0.25"/>']
for d in sorted(paths.keys()):
    t = d / deepest
    color = f"rgb({mix(86, 15, t)},{mix(60, 110, t)},{mix(40, 87, t)})"
    width = max(0.6, 4.2 * (1 - t) ** 1.6)
    out.append(f'<path d="{"".join(paths[d])}" stroke="{color}" stroke-width="{width:.2f}" '
               f'stroke-linecap="round" fill="none"/>')
for i in range(0, len(leaves), 5):
    lx, ly = leaves[i]
    out.append(f'<circle cx="{lx:.1f}" cy="{ly:.1f}" r="1.6" fill="#C99A1E"/>')
out.append("</svg>")
print("\n".join(out))
