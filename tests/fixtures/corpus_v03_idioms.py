# Template idioms of the corpus, compiled natively.
import sys
from sys import stdin, gettrace

# A flag bound once to a literal, and `sys.gettrace()` (no tracer runs in a
# compiled program): the `if` is the branch that runs.
DEBUG = 0
if DEBUG:
    sys.stdin = open("input.txt")

    def read():
        return "debug"
else:

    def read():
        # `next(sys.stdin)`: the next line, `StopIteration` at EOF.
        return next(stdin)[:-1]


if gettrace():

    def show(x):
        print("trace", x)
elif not DEBUG:

    def show(x):
        print(x)


def rl(t=int):
    # A builtin default no call passes is that builtin.
    return list(map(t, read().split()))


def feasible(a, m):
    return min(a[0], a[1]) >= m


def scenes(a, b):
    # Membership in a constant tuple.
    c = (23, 37, 73, 53)
    return a * 10 + b in c


n = int(read())
a = rl()
# A tuple display unpacked: each target is typed.
i, j = min(a), max(a)
m = (i + j) // 2
print(feasible(a, m))
print(scenes(2, 3))
print(scenes(3, 2))
primes = (2, 3, 5, 7, -1)
print([x for x in range(-2, 10) if x in primes])
print([x not in primes for x in a])
# `.sort(reverse=True)` of a subscripted list.
g = [[3, 1, 2], [9, 7, 8]]
for k in range(2):
    g[k].sort(reverse=True)
print(g)
for line in range(n):
    show(read().upper())
try:
    read()
except StopIteration:
    show("eof")
