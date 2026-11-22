"""300 colliding balls drawn with raylib. The physics is native code; each
frame's drawing calls go to raylib through CPython (the `raylib` package).
`-- --frames N` closes the window after N frames."""
import sys

import raylib

from sim import World

COLORS = [raylib.GOLD, raylib.SKYBLUE, raylib.PINK, raylib.LIME, raylib.ORANGE, raylib.VIOLET]


def run(max_frames):
    world = World(960, 600, 300, 42)
    raylib.InitWindow(960, 600, b"pyfast: 300 balls")
    raylib.SetTargetFPS(60)
    frame = 0
    while not raylib.WindowShouldClose() and (max_frames == 0 or frame < max_frames):
        world.step(1.0)
        raylib.BeginDrawing()
        raylib.ClearBackground((18, 20, 30, 255))
        for b in world.balls:
            raylib.DrawCircle(int(b.x), int(b.y), b.r, COLORS[b.hue])
        raylib.DrawText(f"collisions {world.collisions}".encode(), 16, 16, 20, raylib.RAYWHITE)
        raylib.DrawFPS(860, 16)
        raylib.EndDrawing()
        frame += 1
    raylib.CloseWindow()
    return frame


frames = 0
if "--frames" in sys.argv:
    frames = int(sys.argv[sys.argv.index("--frames") + 1])
print("frames drawn:", run(frames))
