import sys

input = sys.stdin.buffer.readline
from math import gcd, log, ceil, sqrt

t = int(input())
for _ in range(t):
    n = int(input())
    a = list(map(int, input().split()))
    # print("processing: ", a)
    g = 0
    if max(a) == 0:
        for i in range(len(a)):
            print(i+1, end = " ")
    else:
        for _ in range(1 + ceil(log(max(a), 2))):
            # print("in if")
            an = 0
            for i in range(len(a)):
                an += a[i] % 2
                a[i] = a[i] // 2
            g = gcd(g, an)
            # print("g is:", g)
        # print("g is ", g)
        if g == 0:
            print(1)
        for k in range(1, g+1):
            if g % k == 0:
                print(k, end=" ")
    print()
