# Non-strict mode: a dict given a key or value of another type (a CPython
# object, a mixed value) is CPython's own dict, which hashes and stores it
# as CPython does.
import json
from collections import deque

visited = {(0, 0): 0}
q = deque([(0, 0)])
steps = json.loads("[[0, 1], [1, 0], [1, 1]]")
while len(q) > 0:
    x, y = q.popleft()
    dist = visited[(x, y)]
    for dx, dy in steps:
        nx, ny = x + dx, y + dy
        if nx <= 2 and ny <= 2 and (nx, ny) not in visited:
            q.append((nx, ny))
            visited[(nx, ny)] = dist + 1
print(sorted(visited.items()))

counts = {"a": 1}
key = json.loads('"b"')
counts[key] = 2
counts["c"] = "three"
print(counts, counts["a"] + counts[key])
try:
    print(counts[json.loads("1")])
except KeyError as e:
    print("KeyError", e)

seen = {}
seen[(1, 2)] = 0
k = json.loads("3")
seen[(k, 4)] = 1
print(seen, (3, 4) in seen)
