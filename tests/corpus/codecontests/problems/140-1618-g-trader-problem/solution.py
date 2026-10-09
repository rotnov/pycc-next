import sys
import bisect
from itertools import *

n, m, q = map(int, sys.stdin.readline().split())
a1 = list(map(int, sys.stdin.readline().split()))
a2 = list(map(int, sys.stdin.readline().split()))
qs = sorted(zip(list(map(int, sys.stdin.readline().split())), range(q)))

nums = a1 + a2
nums.sort()
a1.sort()
sums = [0] + list(accumulate(nums))

nm = n + m
f = list(range(nm))
cnt = [0] * nm

mx = sum(a1)


def find(i):
    if i != f[i]:
        f[i] = find(f[i])
    return f[i]


def union(i, j):
    fi, fj = find(i), find(j)
    if fi != fj:
        if fi < fj:
            fi, fj = fj, fi
        f[fj] = fi
        l1, l2 = cnt[fi], cnt[fj]
        l3 = l1 + l2
        cnt[fi] = l3
        global mx
        mx += sums[fi + 1 - l1] - sums[fi + 1 - l3] - (sums[fj + 1] - sums[fj + 1 - l2])


i, j = 0, 0
while i < n:
    while a1[i] != nums[j]:
        j += 1
    cnt[j] = 1
    i += 1
    j += 1
mg = []
ans = [0] * q
for i in range(nm - 1):
    mg.append((nums[i + 1] - nums[i], i, i + 1))

mg.sort(key=lambda it: it[0], reverse=True)
for k, ai in qs:
    while mg and mg[-1][0] <= k:
        _, i, j = mg.pop()
        union(i, j)
    ans[ai] = mx

print('\n'.join(map(str, ans)))
