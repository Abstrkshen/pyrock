import math


def report(name, score, total):
    pct = score / total
    return f"{name:<8}|{score:>5}|{pct:6.1%}|{total:#x}"


rows = [("ann", 42, 50), ("bob", 7, 9), ("christina", 1234, 2000)]
for r in rows:
    print(report(r[0], r[1], r[2]))
x = 1234567.891
print(f"{x:,.2f} {x:e} {x:.3g} {x:10.1f}| {-x:+.0f} {math.pi:.10f}")
print(f"{255:b} {255:o} {255:X} {65:c} {7:03d} {True} {None} {[1, 2]}")
print(f"{'center':^12}|{'right':*>8}|{3.0}|{1e20}|{1/3!r}")
print("%s has %d items costing %.2f (%5.1f%%) %r" % ("cart", 3, 9.5, 12.25, "x"))
print("%-6s|%6s|%06.2f|%x|%o|%e" % ("ab", "cd", 3.14159, 255, 8, 12345.678))
print("single %s" % 5, "%d%%" % 50)
name = "Ada"
print("Hello, {0}! You are {age} years old. {0!r} {pi:.3f}".format(name, age=36, pi=math.pi))
try:
    print("{} {0}".format(1, 2))
except ValueError as e:
    print("ValueError:", e)
print("{:>{w}}|{:^7.2f}|{{literal}}".format("r", 2.5, w=6))
print(format(3.14159, ".2f"), format(42, "08b"), format("x", "^5"), format(0.5, "%"))
