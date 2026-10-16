# Found by the differential fuzzer (fuzz/): `extend`, `+=` and `insert` on a
# bounded deque ignored `maxlen`.
from collections import deque

d: deque[int] = deque([8, 1, 4, 4], maxlen=3)
d.extend([1, 4])
print(d)
d += [7, 9]
print(d, len(d))
d.extend(d)
print(d)
e: deque[int] = deque([1, 2], maxlen=2)
e.extendleft([5])
print(e)
try:
    e.insert(0, 3)
except IndexError as ex:
    print("IndexError", ex)
print(e)
f: deque[int] = deque([], maxlen=0)
f.extend([1, 2])
print(f)
g: deque[str] = deque(["a", "b", "c"])
g.insert(1, "z")
g.extend(["x"])
print(g)
h: deque[str] = deque(["a"], maxlen=1)
h.extend(["p", "q"])
print(h)
h.insert(0, "r")
