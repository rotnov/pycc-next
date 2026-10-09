import sys;input=sys.stdin.readline
T, = map(int, input().split())
for _ in range(T):
    N, M = map(int, input().split())
    X = [[0]*(M+1)]
    for _ in range(N):
        X.append([0]+[int(c) for c in input().strip()])
    Y = [[0]*(M+1) for _ in range(N+1)]
    for i in range(N+1):
        for j in range(M+1):
            Y[i][j] = X[i][j]
    for i in range(1, N+1):
        for j in range(1, M+1):
            X[i][j] += - X[i-1][j-1] + X[i][j-1] + X[i-1][j]
    R = 10**18
    L = []
    for i in range(5, N+1):
        for j in range(4, M+1):
            x = i-4
            y = j-3
            r = X[i-1][j-1]-X[x][j-1]-X[i-1][y]+X[x][y]
            rrr = (X[i-1][j]+X[i][j-1]-X[i-1][j-1]-r-(X[x-1][j]+X[i][y-1]-X[x-1][y-1]))
            r2 = rrr-Y[x][j]-Y[i][y]-Y[x][y]
            rr = r+(2*(i-x+1-2)+2*(j-y+1-2))-r2
            L.append((rr, x, y))
    L.sort(key=lambda x:x[0])
    for i in range(min(len(L), 23)):
        _, x, y = L[i]
#        print(x, y)
        for i in range(x+4, N+1):
            for j in range(y+3, M+1):
                r = X[i-1][j-1]-X[x][j-1]-X[i-1][y]+X[x][y]
                rrr = (X[i-1][j]+X[i][j-1]-X[i-1][j-1]-r-(X[x-1][j]+X[i][y-1]-X[x-1][y-1]))
                r2 = rrr-Y[x][j]-Y[i][y]-Y[x][y]
                rr = r+(2*(i-x+1-2)+2*(j-y+1-2))-r2
                if R > rr:
                    R = rr
    print(R)
