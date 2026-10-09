t, m = map(int, input().split())
disk = [False] * m
req = 0
for i in range(t):
    inp = input().split()
    if inp[0][0] == "a":
        c = 0
        inp[1] = int(inp[1])
        for j in range(m):
            if disk[j]:
                c = 0
            else:
                c += 1
            if c == inp[1]:
                req += 1
                print(req)
                for j in range(j - inp[1] + 1, j + 1):
                    disk[j] = req
                break
        if c < inp[1]:
            print("NULL")
    elif inp[0][0] == "e":
        inp[1] = int(inp[1])
        if inp[1] > req:
            print("ILLEGAL_ERASE_ARGUMENT")
            continue
        if not inp[1] in disk:
            print("ILLEGAL_ERASE_ARGUMENT")
            continue
        if inp[1] < 1:
            print("ILLEGAL_ERASE_ARGUMENT")
            continue
        for j in range(m):
            if disk[j] == inp[1]:
                disk[j] = False
    elif inp[0][0] == "d":
        for j in range(m):
            if disk[j]:
                _j = j
                while _j > 0 and not disk[_j - 1]:
                    disk[_j - 1] = disk[_j]
                    disk[_j] = False
                    _j -= 1
