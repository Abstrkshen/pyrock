# ASCII Mandelbrot set: nested float loops, chained comparisons, string building.
def escape_time(cr, ci, max_iter):
    zr = 0.0
    zi = 0.0
    for k in range(max_iter):
        zr2 = zr * zr
        zi2 = zi * zi
        if zr2 + zi2 > 4.0:
            return k
        zi = 2.0 * zr * zi + ci
        zr = zr2 - zi2 + cr
    return max_iter


def shade(k, max_iter):
    if k == max_iter:
        return "#"
    if k > 12:
        return "+"
    if k > 6:
        return ":"
    if k > 3:
        return "."
    return " "


rows = 24
cols = 72
max_iter = 200
for y in range(rows):
    line = ""
    for x in range(cols):
        cr = -2.2 + 3.0 * x / cols
        ci = -1.2 + 2.4 * y / rows
        line = line + shade(escape_time(cr, ci, max_iter), max_iter)
    print(line)
