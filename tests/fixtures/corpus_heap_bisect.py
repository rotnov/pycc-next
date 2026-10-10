import heapq
import bisect as bs
from heapq import heappush as hpush, heappop
from bisect import bisect_left, insort

h = []
for v in [5, 1, 4, 1, 3, 9, 2]:
    heapq.heappush(h, v)
print(h)
print([heappop(h) for _ in range(3)], h)
pairs = [(3, "c"), (1, "a"), (2, "b"), (1, "z")]
heapq.heapify(pairs)
print(pairs)
out = []
while pairs:
    out.append(heapq.heappop(pairs))
print(out)
q = [7]
print(heapq.heapreplace(q, 3), q, heapq.heappushpop(q, 1), q, heapq.heappushpop(q, 9), q)
hpush(q, -1)
print(q)
dist = [0.5, 2.5]
heapq.heappush(dist, 1.5)
print(heappop(dist), dist)

a = [1, 2, 2, 4, 7]
print(bisect_left(a, 2), bs.bisect_right(a, 2), bs.bisect(a, 0), bs.bisect_left(a, 8))
insort(a, 3)
bs.insort_left(a, 2)
print(a)
words = ["apple", "fig", "kiwi"]
bs.insort(words, "grape")
print(words, bs.bisect(words, "b"))
empty: list[int] = []
try:
    heapq.heappop(empty)
except IndexError as e:
    print("IndexError", e)
