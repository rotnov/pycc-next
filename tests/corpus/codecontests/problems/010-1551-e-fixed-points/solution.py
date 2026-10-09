def solve(A, k):
    n = len(A)

    dp = [[0] * (n + 1) for _ in range(n + 1)]

    for i in range(n):
        for l in range(1, i + 2):
            dp[i][l] = max(dp[i][l], dp[i - 1][l])
                
            d = 1 if (A[i] == l) else 0
            dp[i][l] = max(dp[i][l], dp[i - 1][l - 1] + d)

    for i in reversed(range(n + 1)):
        if dp[n - 1][i] >= k:
            return n - i

    return -1


T = int(input())
for _ in range(T):
    n, k = map(int, input().split(' '))
    A = list(map(int, input().split(' ')))

    print(solve(A, k))
