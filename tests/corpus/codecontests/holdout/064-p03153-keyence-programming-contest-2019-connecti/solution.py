import sys
read = sys.stdin.buffer.read
readline = sys.stdin.buffer.readline
readlines = sys.stdin.buffer.readlines

from heapq import heappush, heappop, heapify
from collections import defaultdict

"""
・最小値のある場所を調べる。左右にまたがる辺は結ばない。
・最小値の両隣は必ず最小値と結ぶ。
・結んだあと1点に縮約していく。
"""

N,D,*A = map(int,read().split())

A = [0] + A + [0] # 番兵

# (value<<32) + (index)
mask = (1<<32)-1
q = [(x<<32)+i for i,x in enumerate(A[1:-1],1)]
heapify(q)
removed = defaultdict(int)

cost = []
while q:
    while q:
        x = q[0]
        if not removed[x]:
            break
        heappop(q); removed[x] -= 1
    if not q:
        break
    x = heappop(q)
    val,ind = x>>32, x&mask
    L = A[ind-1]; R = A[ind+1]
    if L:
        cost.append(L+val+D)
        # Lの値を書き換える
        newL = val+D
        if L > newL:
            A[ind-1] = newL
            removed[(L<<32)+(ind-1)] += 1
            heappush(q,(newL<<32)+(ind-1))
    if R:
        cost.append(R+val+D)
        # Lの値を書き換える
        newR = val+D
        if R > newR:
            A[ind+1] = newR
            removed[(R<<32)+(ind+1)] += 1
            heappush(q,(newR<<32)+(ind+1))
    A[ind] = 0

answer = sum(cost)
print(answer)
