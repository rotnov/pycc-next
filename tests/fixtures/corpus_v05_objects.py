# v0.5 corpus round, non-strict mode: CPython objects meeting native code.
from itertools import accumulate

# A list that comes to hold values of two types is CPython's own list, and
# `accumulate` over it runs CPython's own.
signs = list("+-+-+")
for i in range(len(signs)):
    signs[i] = 1 if signs[i] == "+" else -1
prefix = [0] + list(accumulate(signs))
print(prefix)

# A generator yielding a native tuple beside CPython objects yields objects;
# `min`/`max` of it, or of a CPython list, run CPython's own.
INF = (10, None)
data = [INF] * 6
for i in range(4):
    data[i] = (5 - i, i)


def window(a, b):
    yield INF
    while a < b:
        yield data[a]
        a += 1


print(min(window(0, 3)), max(window(1, 2)), min(window(3, 3)))
print(min(data), max(data))


# A function returning `None` on one path and a fresh list on another
# returns a CPython object.
def table(n):
    if n % 2 == 0:
        return None
    rows = [["."] * n for _ in range(n)]
    for k in range(n):
        rows[k][k] = "X"
    return rows


def first_even(xs):
    for x in xs:
        if x % 2 == 0:
            return x
    return None


for n in range(1, 5):
    grid = table(n)
    if grid:
        print("\n".join("".join(row) for row in grid))
    else:
        print("none", grid)
print(first_even([3, 5, 8, 9]), first_even([1]))
print(min(window(5, 4)) == INF)
print(max(window(0, 0)))
try:
    print(min(data[6:]))
except ValueError as e:
    print("ValueError:", e)
