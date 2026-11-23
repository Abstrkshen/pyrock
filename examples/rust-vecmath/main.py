# pyrock run main.py
import vecmath


def travel(ax, ay, bx, by):
    return vecmath.length(bx - ax, by - ay)


print("dot:", vecmath.dot(1.0, 2.0, 3.0, 4.0))
print("length:", travel(0.0, 0.0, 3.0, 4.0))
print("lerp:", vecmath.lerp(10.0, 20.0, 0.25))
print("clamp:", vecmath.clamp(12.0, 0.0, 1.0), vecmath.clamp(-2.0, 0.0, 1.0))
print(vecmath.label("vecmath"))
