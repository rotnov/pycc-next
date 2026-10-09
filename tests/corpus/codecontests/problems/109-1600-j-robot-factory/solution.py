
# (north, east, south, west)
directions = [(-1, 0), (0, 1), (1, 0), (0, -1)]

# very sad
i_cant_do_bitwise_operations = {
    0: (False, False, False, False),
    1: (False, False, False, True),
    2: (False, False, True, False),
    3: (False, False, True, True),
    4: (False, True, False, False),
    5: (False, True, False, True),
    6: (False, True, True, False),
    7: (False, True, True, True),
    8: (True, False, False, False),
    9: (True, False, False, True),
    10: (True, False, True, False),
    11: (True, False, True, True),
    12: (True, True, False, False),
    13: (True, True, False, True),
    14: (True, True, True, False),
    15: (True, True, True, True)
}

vis = [] # haha global >:)

def dfs(tiles, i, j):
    vis[i][j] = True

    tot = 1
    for k in range(4):
        if (not tiles[i][j][k] and not vis[i+directions[k][0]][j+directions[k][1]]):
            tot += dfs(tiles, i+directions[k][0], j+directions[k][1])

    return tot


    

if __name__ == "__main__":
    n, m = list(map(int, input().split()))
    tiles = []
    for i in range(n):
        line = input().split()
        tile_line = []
        vis_line = []
        for x in line:
            tile_line.append(i_cant_do_bitwise_operations[int(x)])
            vis_line.append(False)
        tiles.append(tile_line)
        vis.append(vis_line)
    
    rooms = []
    for i in range(n):
        for j in range(m):
            if (not vis[i][j]):
                rooms.append(dfs(tiles, i, j))

    rooms = sorted(rooms, reverse=True)
    for x in rooms:
        print(x, end=" ")
    print()
    

    
