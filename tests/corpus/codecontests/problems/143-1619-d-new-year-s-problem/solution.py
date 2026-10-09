import heapq
import sys
input = sys.stdin.readline

t = int(input())
ans = []
for _ in range(t):
    input()
    m, n = map(int, input().split())
    p = [list(map(int, input().split())) for _ in range(m)]
    if m <= n - 1:
        x = [0] * n
        for pi in p:
            for j in range(n):
                x[j] = max(x[j], pi[j])
        ans0 = min(x)
    else:
        h = []
        for i in range(m):
            pi = p[i]
            for j in range(n):
                heapq.heappush(h, (-pi[j], i, j))
        x = [0] * n
        y = [0] * m
        c = 0
        f1 = 0
        f2 = 0
        while h:
            p0, i, j = heapq.heappop(h)
            if not x[j]:
                x[j] = 1
                c += 1
            if y[i]:
                f1 = 1
            y[i] = 1
            if c == n:
                f2 = 1
            if f1 and f2:
                ans0 = -p0
                break
    ans.append(str(ans0))
sys.stdout.write("\n".join(ans))
