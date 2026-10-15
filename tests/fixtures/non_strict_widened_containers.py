# Non-strict mode: a name holding a native container that meets a CPython
# object becomes an object when every binding of it is a fresh container,
# so aliasing is kept; a list literal nested in an object list, a list
# holding a big `int` literal, `collections.defaultdict` of keys or values
# the native dict cannot hold, and `sys.stdin.buffer` are CPython's own.
import json
import sys
from collections import defaultdict

input = sys.stdin.buffer.readline
o = json.loads("[[1, 2], [3]]")

layer = [0]
while layer != []:
    nxt = []
    for x in layer:
        if x < 2:
            nxt.append(x + 1)
    print(layer)
    layer = nxt

xs = [5, 6]
o.append(xs)
xs.append(7)
print(o, o[2] is xs)


def fresh_row(a, b):
    if a == b:
        return [1 for _ in range(3)]
    row = [0 for _ in range(3)]
    for i in range(3):
        row[i] = (row[i] + a * i + b) % 7
    return row


table = json.loads("{}")
for k in range(3):
    cur = fresh_row(k, 1)
    table[k] = cur
    cur.append(9)
print(table)

mat = [[5, 6], [3, 2]]
o.append(mat)
mat[0].append(1)
print(o[-1])

seen = set()
seen.add(1)
for v in o[0]:
    seen.add(v)
print(sorted(seen))

big = [-1, 1]
big.append(1012809128301279797787789798789798798)
print(big, big[2] > 10**30)

keys = json.loads('["a", "b", "a"]')
by_key = defaultdict(list)
count = defaultdict(int)
none = defaultdict(lambda: None)
for i, key in enumerate(keys):
    by_key[key].append(i)
    count[key] += 1
    print(none[key])
print(sorted(by_key.items()), dict(count), len(none))

line = input()
print(list(line), line.decode().split())
n = int(input())
print(n * 2)
