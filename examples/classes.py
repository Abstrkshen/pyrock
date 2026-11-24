# mode: native-only
from dataclasses import dataclass

@dataclass
class Body:
    x: float
    v: float

    def step(self, dt):
        self.x += self.v * dt

def simulate(bodies, steps):
    for _ in range(steps):
        for b in bodies:
            b.step(0.01)
    return sum(b.x for b in bodies)

bodies = [Body(float(i), 1.0 + i / 10) for i in range(100)]
print(round(simulate(bodies, 1000), 3))
