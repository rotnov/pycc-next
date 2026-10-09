import sys
input = sys.stdin.readline
def gcd(a, b):
    while b: a, b = b, a % b
    return a

N = int(input())
S = 0
Y = []
for i in range(N):
    a, b = map(int, input().split())
    if b > a:
        S += b-a
        Y.append((b, b))
    else:
        Y.append((a, b))

Y = sorted(Y)
YY = [0] * (N+1)
for i in range(N):
    YY[i+1] = YY[i] + Y[i][0]

# i番目を除いてn個選ぶときの余裕度
def f(i, n):
    return S - Y[i][0] + Y[i][1] - (YY[n] if n <= i else YY[n+1] - Y[i][0])

ma1, ma2 = 0, 1
for i in range(N):
    l = 0
    r = N
    while r - l > 1:
        m = (l+r) // 2
        if f(i, m) >= 0:
            l = m
        else:
            r = m

    a = l * Y[i][1] + min(f(i, l), Y[i][1])
    b = N * Y[i][1]
    if a * ma2 > b * ma1:
        ma1, ma2 = a, b

g = gcd(ma1, ma2)
print(ma1//g, ma2//g)
