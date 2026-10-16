# v0.5 corpus round, non-strict mode: CPython's builtins over CPython objects.
import heapq
import itertools

# `range` objects as values, `map(f, <tuple display>)`, and `list` of an
# object.
w = [[1], [2, 3], [4, 5, 6]]
rs = list(map(range, (len(w[0]), len(w[1]), len(w[2]))))
rs[0] = range(len(w[2]) - 1, len(w[2]))
for i in rs[0]:
    for j in rs[1]:
        print(i, j, w[2][i] * 10 + j)
print(list(map(len, ("a", "bc"))), rs[2], range(1, 9, 3))

# `tuple(...)` of a list is CPython's tuple; `sorted`, `sum`, `iter`,
# `next`, `min`/`max` of two and `enumerate` over objects.
pair = tuple(map(int, "4 1 3".split()))
print(pair, sorted(pair), sum(pair), len(pair))
gen = iter([(0, 0), (2, 5)])
a, b = next(gen)
print(a, b)
for a, b in gen:
    print(min(a + 1, b), max(b, a * 7))
try:
    next(gen)
except StopIteration:
    print("StopIteration")
mixed = [None, 4, None]
mixed[0] = 2
for k, v in enumerate(mixed, 1):
    print(k, v)
print([k for k, v in enumerate(mixed) if v is None])

# `itertools.chain` over an object and a list display.
group = [(1, 2), None]
group[1] = (3, 9)
print(list(itertools.chain([0], [g[0] for g in group])))

# A set operator with a fresh set on the right of a CPython object.
t = [1, 2]
for q in [[1, 2], [3]]:
    if 3 in q:
        t = {q.index(3) + 1}
t |= {x + y for x in [10, 20] for y in t}
print(sorted(t))

# A list of `None`s taking ints beside a native graph whose element type
# the CPython list never reaches.
n = 5
graph = {i: [] for i in range(n)}
for i in range(n - 1):
    graph[i].append(i + 1)
    graph[i + 1].append(i)
parents = [None] * n
layer = [0]
children = {}
while layer != []:
    nxt = []
    for guy in layer:
        kids = []
        for boi in graph[guy]:
            if boi != parents[guy]:
                nxt.append(boi)
                kids.append(boi)
                parents[boi] = guy
        children[guy] = kids
    layer = nxt
print(parents, children[3], graph)
pq = []
for guy in children[0]:
    heapq.heappush(pq, (-guy, guy))
print(heapq.heappop(pq), pq)
