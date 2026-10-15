# Non-strict mode: containers that meet CPython objects stay compiled.
from collections import defaultdict, OrderedDict


def chain(x, p):
    q = []
    while x:
        q.append(x)
        x = p[x]
    return q


# A name rebound from a list to a set (on some paths only) is a CPython
# object, and so is the result of an operator over it.
t = list(map(int, "3 0 1 0".split()))
k = 1
groups = [[1, 2], [3], [1]]
for g in groups:
    if k in g:
        t = {g.index(k) + 1}
t |= {10, 20}
for y in t:
    print(y)

# A comprehension over an object whose elements are native lists collects a
# native list of lists.
o = OrderedDict()
o[2] = 0
o[4] = 3
p = [0, 0, 1, 0, 3]
rows = [chain(i, p) for i in o]
print(rows, len(rows[1]), rows[1][0] + 1)
pairs = [(i, i * 2) for i in o if i > 2]
print(pairs)

# `set(o)`, an empty `dict()` of objects, `len(o)` as a key.
print(len(set(o)), len(set(o) | {7}))
names = dict()
for key in o:
    names[key] = str(key)
print(len(names), names[2])
counts = defaultdict(int)
for g in [o, [1], o]:
    counts[len(g)] += 1
print(sorted(counts.items()))

# `x = None` then a native value: the element type still comes from the
# producers.
done = set()
for ch in "hello":
    done.add(ch)
best = None
for el in sorted(done):
    if best is None:
        best = el
print(best, len(done))

# A dict filled through `.get(k, 0)` and decremented through a subscript.
for _ in range(2):
    fnums = {}
    for x in [1, 2, 2, 5]:
        fnums[x] = fnums.get(x, 0) + 1
    for x in range(1, 4):
        if x in fnums:
            fnums[x] = fnums[x] - 1
    remaining = []
    for x, f in fnums.items():
        remaining.extend([x] * f)
    print(sorted(remaining))
zip()
pass
print(format(o[4], ">3"), f"{len(o):03d}")
