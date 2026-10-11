def f(a: int, b: int, c: int) -> int:
    return a * 100 + b * 10 + c


def g(name: str, n: int) -> str:
    return name * n


def pairs(xs: list[int], ys: list[int], limit: int) -> list[tuple[int, int]]:
    return [(x, y) for x in xs if x < limit for y in ys if y != x + 3]


def trace(tag: str, v: int) -> int:
    print("eval", tag, v)
    return v


xs = [1, 2, 3]
ys = [4, 5]
print([x * y for x in xs for y in ys])
print(pairs(xs, ys, 3))
print(sorted({x + y for x in xs for y in ys}))
print({x: [y for y in ys if y > x + 2] for x in xs})
print({str(x) + s: x for x in xs for s in "ab"})
grid = [[1, 2], [3, 4], [5]]
print([v for row in grid for v in row])
print(sum(v for row in grid for v in row))
print(max(v * k for row in grid for v in row for k in range(1, 3)))
print([[a * b for a in row for b in row] for row in grid])
words = ["ab", "c"]
print([ch for w in words for ch in w])
print([(i, j) for i in range(3) for j in range(i)])
print([trace("k", k) for k in range(2) for _ in range(2)])
print([x for x in xs for x in [x, x * 10]])
args = [1, 2, 3]
print(f(*args))
print(f(1, *[2, 3]))
t = (7, 8)
print(f(9, *t))
print(f(*map(int, "4 5 6".split())))
print(g("ab", *[3]))
try:
    print(f(*[1, 2]))
except TypeError as e:
    print("TypeError:", e)
try:
    print(f(1, *[2]))
except TypeError as e:
    print("TypeError:", e)
try:
    print(f(*[1, 2, 3, 4]))
except TypeError as e:
    print("TypeError:", e)
print([v for row in grid for v in row][7])
