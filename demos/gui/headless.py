"""Run the simulation without a window and print a summary (checked in CI)."""
import sys

from sim import World

frames = int(sys.argv[1]) if len(sys.argv) > 1 else 600
world = World(960, 600, 300, 42)
e0 = world.energy()
for _ in range(frames):
    world.step(1.0)
print(f"frames {frames}, balls {len(world.balls)}, collisions {world.collisions}")
print(f"energy {e0:.0f} -> {world.energy():.0f}")
print("resting", sum(1 for b in world.balls if abs(b.vy) < 0.5 and b.y > world.height - b.r - 1))
