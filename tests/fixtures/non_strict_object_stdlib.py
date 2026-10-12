# Non-strict mode: `key=`/`reverse=` sorting and `min`/`max`, and the
# `heapq`/`bisect` functions, over a CPython object run CPython's own.
import heapq
import json
from bisect import bisect_left, insort

r = json.loads("[[3, 5], [1, 9], [2, 4], [0, 4]]")
r.sort(key=lambda t: t[1] - t[0])
print(r)
cards = [tuple(c) for c in json.loads("[[1, 2], [3, 1], [1, 5]]")]
cards.sort(reverse=True)
print(cards)
rooms = json.loads("[4, 9, 1, 9]")
rooms = sorted(rooms, reverse=True)
print(rooms, sorted(json.loads('["bb", "a", "ccc"]'), key=len))
words = json.loads('["pear", "fig", "apple", "kiwi"]')
print(min(words, key=len), max(words, key=len))
try:
    min(json.loads("[]"), key=len)
except ValueError as e:
    print("ValueError", e)

h = json.loads("[]")
for v in [5, 1, 4, 1]:
    heapq.heappush(h, (v, v * 2))
print(heapq.heappop(h), h)
a = json.loads("[1, 3, 3, 7]")
print(bisect_left(a, 3))
insort(a, 4)
print(a)
try:
    heapq.heappush(h, "x")
except TypeError as e:
    print("TypeError", e)
