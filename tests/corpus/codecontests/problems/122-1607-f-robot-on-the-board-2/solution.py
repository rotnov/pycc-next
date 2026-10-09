from sys import stdin

input = stdin.readline

moves = {
    "R": (0, 1),
    "L": (0, -1),
    "U": (-1, 0),
    "D": (1, 0)
}

lengths = []

def dumbdfs(x, y):
    global lengths
    path = []
    lenn = 0
    curx = x
    cury = y
    cycle = False
    cycle_start = 0

    while curx >= 0 and curx < n and cury >= 0 and cury < m:

        if lengths[curx][cury] == -1:
            cycle_start = path.index((curx, cury))
            cycle = True
            break

        if lengths[curx][cury] > 0:
            lenn += lengths[curx][cury]
            break

        lenn += 1

        lengths[curx][cury] = -1
        path.append((curx, cury))

        dx, dy = moves[field[curx][cury]]
        curx += dx
        cury += dy

    if cycle:
        for i in range(lenn, lenn-cycle_start, -1):
            lengths[path[lenn - i][0]][path[lenn - i][1]] = i
        for i in range(cycle_start, len(path)):
            lengths[path[i][0]][path[i][1]] = lenn-cycle_start
    else:
        for i in range(lenn, lenn - len(path), -1):
            lengths[path[lenn - i][0]][path[lenn - i][1]] = i

t = int(input())
for _ in range(t):
    input()
    n, m = map(int, input().split(" "))
    field = [input() for _ in range(n)]
    lengths = [[0 for _ in range(m)] for _ in range(n)]

    for x in range(n):
        for y in range(m):
            if not lengths[x][y]:
                dumbdfs(x, y)

    maxx = 0
    maxy = 0
    maxv = 0
    for x in range(n):
        for y in range(m):
            if lengths[x][y] > maxv:
                maxv = lengths[x][y]
                maxx = x + 1
                maxy = y + 1
    
    print(maxx, maxy, maxv)

"""

2 2
UD
RU

"""
