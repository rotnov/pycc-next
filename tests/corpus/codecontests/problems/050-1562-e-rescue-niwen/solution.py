import sys
input = sys.stdin.readline


for _ in range(int(input())):
    n = int(input())
    s = input()[:-1]
    lcp = [[0] * (n + 1) for _ in range(n + 1)]
    for i in range(n - 1, -1, -1):
        for j in range(n - 1, -1, -1):
            if s[i] == s[j]: lcp[i][j] = lcp[i + 1][j + 1] + 1

    def cal(x, y):
        t = lcp[x][y]
        if y + t >= n: return -1
        xc, yc = s[x + t], s[y + t]
        if xc > yc: return -1
        return t

    dp = [0] * n
    for i in range(n):
        dp[i] = n - i
        for j in range(i):
            x = cal(j, i)
            if x != -1:
                dp[i] = max(dp[i], dp[j] + n - i - x)
    print(max(dp))
