from collections import defaultdict as dd, deque
def check_cycle(g, indegree, n):
    c = 0
    d = deque([])
    ans = dd(lambda :1)
    for i in range(n):
        if indegree[i + 1] == 0:
            d.append(i + 1)
    #print(d)
    while d:
        u = d.popleft()
        c += 1
        for v in g[u]:
            indegree[v] -= 1
            if v < u:
                ans[v] = max(ans[v], ans[u] + 1)
            else:
                ans[v] = max(ans[v], ans[u])
            if indegree[v] == 0:
                d.append(v)
    if c != n:
        return False
    if ans:
        return max(ans.values())
    return 1


for _ in range(int(input())):
    n = int(input())
    g = dd(set)
    indegree = dd(int)
    for i in range(n):
        a = list(map(int, input().split()))
        for j in range(1, a[0] + 1):indegree[i + 1] += 1;g[a[j]].add(i + 1)
    res = check_cycle(g, indegree, n);print(-1) if not res else print(res)
