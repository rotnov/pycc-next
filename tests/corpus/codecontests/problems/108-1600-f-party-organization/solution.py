import sys
input = sys.stdin.readline

n,m = map(int,input().split())

G = [[] for _ in range(n)]
for _ in range(m):
    u,v = map(int,input().split())
    u -= 1
    v -= 1

    if u >= 48 or v >= 48:
        continue

    G[u].append(v)
    G[v].append(u)

if n >= 48:
    G = G[:48]

def ok(a,b,c,d,e):
    allfd = True
    nofd = True

    people = [a,b,c,d,e]
    for x in people:
        for y in people:
            if x == y:
                continue
            if x in G[y]:
                nofd = False
            else:
                allfd = False

    return allfd or nofd

sz = len(G)
for a in range(sz):
    for b in range(a+1,sz):
        for c in range(b+1,sz):
            for d in range(c+1,sz):
                for e in range(d+1,sz):
                    if ok(a,b,c,d,e):
                        ans = [a,b,c,d,e]
                        print(*[x+1 for x in ans])
                        exit()

print(-1)
