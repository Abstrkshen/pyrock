# mode: native-only
# The playground's "life" example (site/play.html): a picture printed as text.
# Several P3 images in a row play as an animation: Conway's Game of Life,
# one frame per generation.
W, H = 48, 32

def step(grid):
    new = []
    for y in range(H):
        row = []
        for x in range(W):
            n = 0
            for dy in (-1, 0, 1):
                for dx in (-1, 0, 1):
                    if dx != 0 or dy != 0:
                        n += grid[(y + dy) % H][(x + dx) % W]
            alive = grid[y][x] == 1
            row.append(1 if n == 3 or (alive and n == 2) else 0)
        new.append(row)
    return new

seed = 12345
def rand():
    global seed
    seed = (seed * 1103515245 + 12345) % 2147483648
    return seed

grid = [[1 if (rand() >> 16) % 3 == 0 else 0 for _ in range(W)] for _ in range(H)]  # high bits: the low ones repeat
for gen in range(60):
    print("P3", W, H, 9)  # colour values from 0 to 9
    for row in grid:
        print(" ".join("9 7 1" if c else "1 2 2" for c in row))
    grid = step(grid)
