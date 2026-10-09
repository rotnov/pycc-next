from sys import setrecursionlimit

from bisect import bisect_left, bisect_right
from collections import deque
from functools import lru_cache, reduce
from heapq import heappush, heappop
from math import sqrt, ceil, floor, log2

T = int(input())

def rl(t = int):
    return list(map(t, input().split()))

for t in range(1, T + 1):
    n = int(input())
    r = []
    for _ in range(n):
        r.append(rl())

    ret = []
    have = set()
    r.sort(key = lambda t: t[1] - t[0])
    for s, e in r:
        for i in range(s, e + 1):
            if i not in have:
                break
        have.add(i)
        ret.append((s, e, i))

    for el in ret:
        print(' '.join(map(str, el)))
    print('\n')
