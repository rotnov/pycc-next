from collections import deque
while True:
    n=int(input())
    if n==0:
        exit()
    data = []
    for i in range(n):
        tmp = list(map(int,input().split()))
        tmp = [tmp[:4],tmp[4:8],tmp[8:12],tmp[12:]]
        d = [ [0 for i in range(3)]for i in range(3)]
        for i in range(3):
            for j in range(3):
                for k in range(2):
                    for l in range(2):
                        d[i][j] += tmp[i+k][j+l]
        data.append(d)
    
    q = deque()
    memo = set()
    if data[0][1][1]:
        print(0)
        continue
    q.append((0,1,1,(0,0,0,0)))
    while len(q):
        z,y,x,h = q.popleft()
        if (y,x) == (0,0):
            h = (0,h[1]+1,h[2]+1,h[3]+1)
        elif (y,x) == (0,2):
            h = (h[0]+1,0,h[2]+1,h[3]+1)
        elif (y,x) == (2,0):
            h = (h[0]+1,h[1]+1,0,h[3]+1)
        elif (y,x) == (2,2):
            h = (h[0]+1,h[1]+1,h[2]+1,0)
        else:
            h = (h[0]+1,h[1]+1,h[2]+1,h[3]+1)
        if max(h)>6:
            continue
        if (z,y,x,h) in memo:
            continue
        memo.add((z,y,x,h))
        if z==n-1:
            print(1)
            break
        for i in range(3):
            if not data[z+1][i][x]:
                q.append((z+1,i,x,h))
            if not data[z+1][y][i]:
                q.append((z+1,y,i,h))
    else:
        print(0)

        
