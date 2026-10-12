def f(a: int, b: int = 2, *args: int, c: int = 3, **kw: int) -> int:
    print(a, b, args, c, kw)
    return a + b + c + sum(args) + len(kw)


def only_kw(*, x: int, y: str = "y") -> str:
    return str(x) + y


def pos(a: int, b: int, /, c: int = 0, *, d: int = 0) -> int:
    return a * 100 + b * 10 + c


def gather(*names: str) -> str:
    print(len(names), names)
    for n in names:
        print("-", n)
    if names:
        print("first", names[0], "rest", names[1:])
    return ", ".join(names)


def options(**opts: float) -> float:
    total = 0.0
    for k in opts:
        print(k, opts[k])
        total += opts[k]
    return total


def accumulate(x: int, acc: list[int] = []) -> list[int]:
    acc.append(x)
    return acc


print(f(1))
print(f(1, 5, 6, 7, c=9, x=1, y=2))
print(f(c=10, a=1))
print(only_kw(x=4))
print(only_kw(y="z", x=5))
print(pos(1, 2))
print(pos(1, 2, c=3))
print(gather())
print(gather("a"))
print(gather("a", "b", "c"))
print(options())
print(options(alpha=1.5, beta=2.0))
print(accumulate(1))
print(accumulate(2))
print(accumulate(3, []))

d: dict[str, int] = {"b": 5, "c": 6}
print(f(1, **d))
xs: list[int] = [1, 2, 3, 4]
print(f(*xs))
print(f(*xs, c=0, z=9))
ws: list[str] = ["p", "q"]
print(gather(*ws))
print(pos(*[7, 8]))
m: dict[str, float] = {"u": 1.0}
print(options(**m))
print(options(v=3.0, **m))

for bad in range(9):
    try:
        if bad == 0:
            f()
        elif bad == 1:
            only_kw()
        elif bad == 2:
            only_kw(1)
        elif bad == 3:
            pos(1, b=2)
        elif bad == 4:
            f(1, a=2)
        elif bad == 5:
            only_kw(x=1, z=2)
        elif bad == 6:
            pos(1, 2, 3, 4)
        elif bad == 7:
            f(**{"a": 1, "q": 3, "c": 4})
        else:
            pos(*[1])
    except TypeError as e:
        print("TypeError:", e)


class Point:
    def __init__(self, x: int = 0, y: int = 0, *, label: str = "p") -> None:
        self.x = x
        self.y = y
        self.label = label

    def moved(self, dx: int = 0, dy: int = 0, *extra: int, scale: int = 1) -> "Point":
        print("extra", extra)
        return Point(self.x + dx * scale, self.y + dy * scale, label=self.label)

    def describe(self, *, prefix: str = "") -> str:
        return prefix + self.label + "(" + str(self.x) + ", " + str(self.y) + ")"

    @staticmethod
    def origin(*, label: str = "o") -> "Point":
        return Point(label=label)


def trace(tag: str) -> int:
    print("eval", tag)
    return len(tag)


def order(a: int, b: int, *, c: int) -> str:
    return str(a) + str(b) + str(c)


def counter(start: int = 0, step: int = 1, *, limit: int = 3) -> list[int]:
    out: list[int] = []
    v = start
    while len(out) < limit:
        out.append(v)
        v += step
    return out


def outer(n: int) -> int:
    def inner(a: int, *, scale: int = 2) -> int:
        return a * scale + n
    return inner(1) + inner(2, scale=10)


p = Point(1, 2)
print(p.describe())
print(p.describe(prefix="> "))
q = p.moved(1, 1, 9, 8, scale=3)
print(q.describe())
print(Point(y=5).describe())
print(Point(label="z", x=7).describe())
print(Point.origin().describe())
print(Point.origin(label="O").describe())
print(order(b=trace("b"), a=trace("aa"), c=trace("ccc")))
print(order(trace("1"), c=trace("33"), b=trace("222")))
print(counter())
print(counter(5, limit=2))
print(counter(step=2, start=1))
print(outer(100))
try:
    Point(1, 2, 3)
except TypeError as e:
    print(e)
try:
    p.moved(scale=1, dx=1, dy=2, z=3)
except TypeError as e:
    print(e)
try:
    p.describe("x")
except TypeError as e:
    print(e)


def show(*args: int) -> str:
    return f"{args} {str(args)} {len(args)} {sum(args)} {args[-1] if args else -1}"


def pair(a: int, b: int = 10, /) -> int:
    return a + b


def kws(a: int, b: int, c: int = 0) -> int:
    return a * 100 + b * 10 + c


def var_kw(x: str, **rest: str) -> str:
    out = x
    for k in rest:
        out += "," + k + "=" + rest[k]
    return out


print(show())
print(show(5))
print(show(1, 2, 3))
nums: list[int] = [4, 5, 6]
print(show(*nums))
print(show(0, *nums))
print(pair(*[1]))
print(pair(*[1, 2]))
print(kws(**{"a": 1, "b": 2}))
print(kws(1, **{"c": 3, "b": 2}))
print(kws(*[1, 2], **{"c": 9}))
print(var_kw("x", **{"p": "1", "q": "2"}))
print(var_kw(**{"x": "y"}))
print(var_kw("x", z="0", **{"p": "1"}))

cases: list[dict[str, int]] = [{"a": 1}, {"a": 1, "b": 2, "d": 4}, {"b": 2, "c": 3}, {}]
for case in cases:
    try:
        print(kws(**case))
    except TypeError as e:
        print("TypeError:", e)

seqs: list[list[int]] = [[], [1], [1, 2, 3], [1, 2, 3, 4]]
for seq in seqs:
    try:
        print(pair(*seq))
    except TypeError as e:
        print("TypeError:", e)
    try:
        print(kws(*seq))
    except TypeError as e:
        print("TypeError:", e)
    try:
        print(kws(*seq, b=7))
    except TypeError as e:
        print("TypeError:", e)

try:
    print(kws(1, **{"a": 5, "b": 2}))
except TypeError as e:
    print("TypeError:", e)
try:
    print(kws(1, b=3, **{"b": 2}))
except TypeError as e:
    print("TypeError:", e)
try:
    print(pair(**{"a": 1}))
except TypeError as e:
    print("TypeError:", e)
