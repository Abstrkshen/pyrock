# mode: native-only
# Carves a random maze (depth-first, with a seeded generator), then solves it
# with breadth-first search. Prints P3 frames: the search spreading out from
# the entrance, then the shortest path.
# pyrock --release: 5 ms; CPython 3.12: 34 ms (whole run, this machine)
CW, CH = 25, 17            # maze size in cells
W, H = 2 * CW + 1, 2 * CH + 1  # image size: walls between cells

seed = 2026


def rand(n):
    global seed
    seed = (seed * 1103515245 + 12345) % 2147483648
    return (seed >> 16) % n  # the high bits: the low ones repeat quickly


# 1 = wall, 0 = open
grid = [[1 for _ in range(W)] for _ in range(H)]
visited = [[False for _ in range(CW)] for _ in range(CH)]
stack = [(0, 0)]
visited[0][0] = True
grid[1][1] = 0
while stack:
    cx, cy = stack[-1]
    options = []
    for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
        nx, ny = cx + dx, cy + dy
        if 0 <= nx < CW and 0 <= ny < CH and not visited[ny][nx]:
            options.append((nx, ny))
    if not options:
        stack.pop()
        continue
    nx, ny = options[rand(len(options))]
    visited[ny][nx] = True
    grid[cy + ny + 1][cx + nx + 1] = 0  # the wall between the two cells
    grid[2 * ny + 1][2 * nx + 1] = 0
    stack.append((nx, ny))

start = (1, 1)
goal = (W - 2, H - 2)

# Breadth-first search, remembering when each square was reached.
dist = [[-1 for _ in range(W)] for _ in range(H)]
prev = [[-1 for _ in range(W)] for _ in range(H)]
dist[1][1] = 0
queue = [start]
head = 0
while head < len(queue):
    x, y = queue[head]
    head += 1
    if (x, y) == goal:
        break
    for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
        nx, ny = x + dx, y + dy
        if grid[ny][nx] == 0 and dist[ny][nx] < 0:
            dist[ny][nx] = dist[y][x] + 1
            prev[ny][nx] = y * W + x
            queue.append((nx, ny))

path = set()
at = goal[1] * W + goal[0]
while at >= 0:
    path.add(at)
    at = prev[at // W][at % W]

# Colours on a 0..9 scale (P3 with maximum value 9 keeps the text small).
WALL = "2 2 2"
OPEN = "9 9 9"
SEEN = "1 4 3"
FRONT = "8 6 1"
PATH = "8 6 1"
END = "3 5 8"

longest = dist[goal[1]][goal[0]]
steps = 48
for f in range(steps + 12):
    limit = longest * min(f, steps) // steps
    done = f >= steps
    print("P3", W, H, 9)
    for y in range(H):
        row = []
        for x in range(W):
            d = dist[y][x]
            if grid[y][x] == 1:
                row.append(WALL)
            elif (x, y) == start or (x, y) == goal:
                row.append(END)
            elif done and (y * W + x) in path:
                row.append(PATH)
            elif d >= 0 and d == limit:
                row.append(FRONT)
            elif d >= 0 and d < limit:
                row.append(SEEN)
            else:
                row.append(OPEN)
        print(" ".join(row))
