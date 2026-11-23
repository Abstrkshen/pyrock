# pyrock run main.py
# kern.c is compiled into the binary. wave.cpp is a shared library:
#   g++ -shared -fPIC -O2 -o libwave.so wave.cpp
import kern
import wave


def track(steps):
    pos = 2
    vel = 3
    line = ""
    i = 0
    while i < steps:
        vel = kern.step_vel(pos, vel, 0, 12)
        pos = kern.step_pos(pos, vel, 0, 12)
        line = line + str(pos) + " "
        i = i + 1
    return line


print("c language:", kern.language())
print("c lerp:", kern.lerp(0.0, 100.0, 0.25))
print("c track:", track(8))
print("cpp sine 0.00:", wave.sample(0.0))
print("cpp sine 0.25:", wave.sample(0.25))
print("cpp sine 0.50:", wave.sample(0.5))
