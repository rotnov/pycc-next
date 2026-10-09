
"""

from
https://atcoder.jp/contests/arc115/submissions/27470789

"""

import sys
from sys import stdin

input = sys.stdin.readline
N = int(input())
a = list(map(int, input().split()))
mod = 998244353

dp = [0] * (N + 1)
dp[0] = 1
s = []
sm = 0
for i in range(N):
    x = a[i]

    c = dp[i]
    if i & 1: c = -dp[i]

    while len(s) and s[-1][0] >= x:
        v, cc = s.pop()
        c += cc
        c %= mod
        sm -= v * cc
        sm %= mod

    s.append((x, c))
    sm += x * c
    sm %= mod

    if i & 1: dp[i + 1] -= sm
    else: dp[i + 1] += sm
    dp[i + 1] %= mod

print(dp[-1])
