# pyrock run main.py
# pyrock check main.py --mode native-only

def escape(cx, cy):
    zx = 0.0
    zy = 0.0
    i = 0
    while i < 40:
        next_x = zx * zx - zy * zy + cx
        zy = 2.0 * zx * zy + cy
        zx = next_x
        if zx * zx + zy * zy > 4.0:
            return i
        i = i + 1
    return 40


def shade(n):
    if n >= 40:
        return "#"
    if n >= 20:
        return "+"
    if n >= 8:
        return "."
    return " "


def row(y, height, width):
    line = ""
    x = 0
    while x < width:
        cx = -2.5 + (x / width) * 3.5
        cy = -1.2 + (y / height) * 2.4
        line = line + shade(escape(cx, cy))
        x = x + 1
    return line


def picture(height, width):
    y = 0
    while y < height:
        print(row(y, height, width))
        y = y + 1


picture(24, 64)
