# Non-strict mode: a fresh native list (nothing else holds it yet) crosses
# into CPython code as a copy, so it can be passed to a library call, kept
# in a CPython container or read by fallback code.
from collections import Counter, deque
from itertools import accumulate, combinations


class P:
    def __init__(self, x: int):
        self.x = x


xs = list(map(int, "3 1 2 3 1".split()))
print(sorted(Counter([x * 2 for x in xs]).items()))
d = deque([(i, 0) for i in range(3)])
d.appendleft((9, 9))
print(list(d))
print(list(combinations(list(range(4)), 2)))
print(list(accumulate(sorted(xs))))
print(list(accumulate([1.5, 2.5])), list(accumulate([True, False, True])))
print(Counter("a b a".split()).most_common(1))
grid = deque([[0] * 2 for _ in range(2)])
grid[0][1] = 5
print(list(grid))
ps = deque([P(1), P(2)])
print([p.x for p in ps])
print(list(reversed(xs[1:4])), deque(xs + [7]))
print(Counter(xs * 2)[3])
