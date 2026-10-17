"""`itertools.takewhile` and `itertools.dropwhile` with a lambda or a
function predicate, compiled natively."""

import itertools
from itertools import dropwhile, takewhile


def small(a: int) -> bool:
    return a < 3


xs = [1, 2, 5, 1, 0]
words = ["ab", "c", "def", "", "g"]
print(list(itertools.takewhile(lambda a: a < 3, xs)), list(itertools.dropwhile(lambda a: a < 3, xs)))
print(list(takewhile(small, xs)), list(dropwhile(small, xs)))
print(list(takewhile(lambda w: w, words)), list(dropwhile(lambda w: len(w) < 3, words)))
print(list(takewhile(lambda a: a > 9, xs)), list(dropwhile(lambda a: a > -1, xs)))
print(sum(takewhile(lambda a: a % 7 != 6, range(100))), [p for p in dropwhile(lambda p: p[0] < 2, [(1, "a"), (2, "b"), (0, "c")])])
for v in takewhile(lambda a: a < 5, itertools.count(1)):
    print("v", v)
