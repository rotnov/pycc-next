# Non-strict: globals fallback code reads that hold lists live in the module
# namespace as CPython lists, built there directly when the value is a fresh
# display; a lambda alias fallback code calls stays compiled too.
from sys import stdin

mi = lambda: map(int, stdin.readline().split())
n, m = mi()
fact = [1, 1]
dp = [0] * (n + 1)
grid = [[0] * m] * 2
names = []
names.append(n)
names.append("x")


def gen():
    # Falls back (a generator); reads the globals above.
    for i in range(len(fact)):
        yield fact[i] + dp[i] + len(names) + sum(mi())


for v in gen():
    print(v)
fact.append(2)
dp[1] = 7
grid[0][0] = 5
print(fact, dp, grid, names)
print(list(gen()))
