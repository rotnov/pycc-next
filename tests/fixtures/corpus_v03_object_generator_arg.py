# Non-strict mode: a generator passed to a CPython object's method crosses
# as an iterator over it.
import itertools
import json

close = json.loads("[]")
for g in [[1, 2, 3], [2, 5]]:
    close.extend(itertools.combinations(g, 2))
    close.extend(x * 2 for x in g)
print(close)
