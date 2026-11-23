# mode: native-only
# The playground's "mandel" example (site/play.html): a picture printed as text.
# Print a "P3" image (plain-text PPM: width, height, then r g b per pixel)
# and the playground paints it on a canvas.
W, H, LIMIT = 180, 120, 80

def escape(cr, ci):
    zr = 0.0
    zi = 0.0
    for i in range(LIMIT):
        zr, zi = zr * zr - zi * zi + cr, 2 * zr * zi + ci
        if zr * zr + zi * zi > 4.0:
            return i
    return LIMIT

def mix(a, b, t):
    return int(a + (b - a) * t)

def color(n):
    if n == LIMIT:
        return "28 35 38"
    s = (n / LIMIT) ** 0.45
    if s < 0.6:  # pale stone to malachite
        u = s / 0.6
        return f"{mix(236, 15, u)} {mix(238, 110, u)} {mix(233, 87, u)}"
    u = (s - 0.6) / 0.4  # malachite to pyrite
    return f"{mix(15, 201, u)} {mix(110, 154, u)} {mix(87, 30, u)}"

print("P3")
print(W, H)
print(255)
for y in range(H):
    row = []
    for x in range(W):
        cr = -2.2 + 3.1 * x / W
        ci = -1.2 + 2.4 * y / H
        row.append(color(escape(cr, ci)))
    print(" ".join(row))
