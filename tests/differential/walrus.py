# Assignment expressions (`:=`) in conditions, loops and module code.
import re

LINES = ["alpha 1", "", "beta 22", "gamma 333", ""]


def parse(lines):
    out = []
    i = 0
    while (n := len(lines) - i) > 0:
        line = lines[i]
        i += 1
        if (parts := line.split()) and len(parts) == 2:
            out.append((parts[0], int(parts[1]), n))
    return out


def first_long(words, k):
    for w in words:
        if (size := len(w)) > k:
            return w, size
    return "", 0


def matches(text):
    found = []
    if (m := re.search(r"(\d+)", text)) is not None:
        found.append(m.group(1))
    return found


print(parse(LINES))
print(first_long(["a", "abc", "abcdef"], 2), first_long([], 1))
print(matches("x 42 y"), matches("none"))
if (total := sum(len(x) for x in LINES)) > 10:
    print("total", total)
values = [y for x in [1, 2, 3] if (y := x * 10) > 10]
print(values, y)
