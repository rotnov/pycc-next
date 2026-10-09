n,m = map(int,input().split())
a_list = [int(input()) for _ in range(m)]

dp = [1]*(n+1)

for a in a_list:
    dp[a] = 0

for i in range(2,n+1):
    if dp[i] != 0:
        dp[i] = dp[i-1] + dp[i-2]

print(dp[n]%(10**9+7))
