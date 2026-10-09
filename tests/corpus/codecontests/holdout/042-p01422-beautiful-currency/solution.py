# coding:utf-8

import sys


input = sys.stdin.readline
INF = float('inf')
MOD = 10 ** 9 + 7


def inpl(): return list(map(int, input().split()))


def solve(N):
    A = inpl()
    dp = [INF] * (A[0] * 2)
    for i in range(A[0]//2, A[0]*2):
        dp[i] = abs(i - A[0]) / A[0]  # A[0]の価格を変えたときのconfusion ratio

    for i in range(N-1):
        a1 = A[i + 1]
        nn = a1 * 2
        ndp = [INF] * nn  # ndp[A[i+1]の変更後の価値] = confusion ratioの最小値
        for j in range(1, len(dp)):
            if dp[j] == INF:
                continue

            t = dp[j]
            for k in range(j, nn, j):
                u = abs(a1 - k) / a1
                if u < t:  # A[i]とA[i+1]でconfusion ratioの大きい方をndpと比較
                    u = t
                if ndp[k] > u:  # A[1]~A[i+1]まで帳尻を合わせたときのconfusion ratioの最小値を格納
                    ndp[k] = u

        dp = ndp

    return '{:0.12f}'.format(min(dp))


N = int(input())
if N != 0:
    print(solve(N))
