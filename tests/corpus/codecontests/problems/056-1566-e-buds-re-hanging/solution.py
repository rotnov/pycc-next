import sys

input = sys.stdin.readline

for _ in range(int(input())):
    n = int(input())
    edges = [[] for _ in range(n)]
    for _ in range(n - 1):
        u, v = map(int, input().split())
        edges[u - 1].append(v - 1)
        edges[v - 1].append(u - 1)

    pare, fills, pendent, nivells = {0: None}, [set() for _ in range(n)], [0], []
    while pendent:
        nou_pendent = []
        for p in pendent:
            for f in edges[p]:
                if f != pare[p]:
                    pare[f] = p
                    fills[p].add(f)
                    nou_pendent.append(f)
        pendent = nou_pendent
        nivells.append(pendent)

    buds = []
    for nivell in reversed(nivells):
        for b in nivell:
            if len(fills[b]) > 0 and all(len(fills[f])==0 for f in fills[b]):
                buds.append(len(fills[b]))
                p = pare[b]
                fills[p].remove(b)

    ans = max(len(fills[0]), 1)
    for b in buds:
        ans += b-1
    print(ans)
