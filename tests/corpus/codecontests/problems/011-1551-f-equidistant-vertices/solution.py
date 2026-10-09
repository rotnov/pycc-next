import sys
input = sys.stdin.readline
from collections import deque, Counter

mod = 10 ** 9 + 7
for _ in range(int(input())):
    input()
    n, k = map(int, input().split())
    G = [[] for _ in range(n)]
    for _ in range(n - 1):
        a, b = map(int, input().split())
        a -= 1
        b -= 1
        G[a].append(b)
        G[b].append(a)

    if k == 2:
        print(n * (n - 1) // 2 % mod)
        continue

    ans = 0
    for r in range(n):
        cnt = [0] * n
        dp = [[0] * (k + 1) for _ in range(n)]
        for i in range(1, n):
            dp[i][0] = 1

        def dfs(i, par, d):
            cnt[d] += 1
            for j in G[i]:
                if j == par: continue
                dfs(j, i, d + 1)

        for x in G[r]:
            cnt = [0] * n
            dfs(x, r, 1)
            for i in range(1, n):
                if cnt[i] == 0: break
                for j in range(k, 0, -1):
                    dp[i][j] += dp[i][j - 1] * cnt[i]
                    dp[i][j] %= mod

        for i in range(1, n):
            ans += dp[i][k]
            ans %= mod

    print(ans)
