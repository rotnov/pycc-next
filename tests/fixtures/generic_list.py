class Point:
    def __init__(self, x: int, y: float) -> None:
        self.x = x
        self.y = y


def _total(xs: list[float]) -> float:
    s = 0.0
    for x in xs:
        s += x
    return s


def _doubled(xs: list[int]) -> list[int]:
    return [x * 2 for x in xs]


def _words(n: int) -> list[str]:
    out: list[str] = []
    for i in range(n):
        out.append(f"w{i}")
    return out


def _grid(rows: int, cols: int) -> list[list[int]]:
    g: list[list[int]] = []
    for r in range(rows):
        g.append([r * cols + c for c in range(cols)])
    return g


def _bump(points: list[Point]) -> None:
    for p in points:
        p.x += 1


names: list[str] = []
names.append("ann")
names.append("bob")
print(names, len(names), names[-1])

# float
fs = [1.5, 2.25, -3.0]
fs.append(4.0)
fs[1] = 0.5
print(fs, fs[0], fs[-1], len(fs), _total(fs))
print(fs[1:3], fs[::-1], fs[:-1], fs[-2:])
print(2.25 in fs, 0.5 in fs, 7.0 not in fs)

# bool
flags = [True] * 3
flags[1] = False
print(flags, flags + [False], 2 * [False])
count = 0
for flag in flags:
    if flag:
        count += 1
print(count)

# int
ints = [3, 1, 2]
ints.insert(0, 9)
ints.insert(-1, 7)
ints.insert(100, 5)
print(ints, ints.pop(), ints.pop(0), ints)
print(_doubled(ints), ints * 0, [0] * 4, 3 in ints)
mixed: list[int] = [1]
mixed.append(True)
print(mixed, f"{mixed}", str(mixed), str(fs) + "!")

# str
ss = ["a", "it's", 'say "hi"', "tab\there"]
ss.append("d" + "e")
first = ss[0]
ss[0] = "z"
print(first, ss, len(ss))
print("de" in ss, "q" in ss, ss[1:3], ss.pop(), ss)
print(_words(3), _words(0))
two = _words(2)
for w in two:
    print(w)
print(f"{ss}")

# nested
g = _grid(2, 3)
print(g, g[1][2], len(g[0]))
top = g[0]
top[0] = 99
row = g[1]
row.append(100)
rows = [[1], [2]]
print(g, [1] in rows, [3] in rows, [[0] * 2] * 2)
empty: list[list[float]] = [[]]
print(empty, empty[0])

# class instances alias like CPython
pts = [Point(1, 0.5), Point(2, 1.5)]
alias = pts
other = pts[0]
pts[0].x = 10
_bump(pts)
print(other.x, alias[1].x, len(pts), pts[-1].y)
pts2 = pts + [Point(5, 5.0)]
pts2[0].x = 42
print(pts[0].x, len(pts2), other in pts, pts2[2] in pts)

# truthiness and comprehension over lists
if names:
    print("non-empty")
cleared: list[int] = []
if not cleared:
    print("empty")
print([s + "!" for s in names if s != "bob"], [p.x for p in pts])

# IndexError messages
try:
    print(ints[10])
except IndexError as e:
    print("IndexError:", e)
try:
    ints[-10] = 1
except IndexError as e:
    print("IndexError:", e)
try:
    cleared.pop()
except IndexError as e:
    print("IndexError:", e)
try:
    ints.pop(50)
except IndexError as e:
    print("IndexError:", e)
try:
    print(ints[::0])
except ValueError as e:
    print("ValueError:", e)
print(round(2.675, 2), round(-0.5), round(1.5), round(7), round(1234.5678, -2))
print(fs[5])
