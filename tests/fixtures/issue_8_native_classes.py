# Issue #8: in a --non-strict native build, a module-level statement that
# runs in CPython (here: each one calls `eval`) keeps the module's compiled
# classes native when Python code can use every member of theirs: fallback
# code constructs them, calls their methods, reads and stores their fields
# (a list field included, shared with compiled code), compares, sorts and
# prints them. A class with an unannotated method (`Loose`) still runs in
# CPython, as fallback code could not call it compiled.


class Point:
    """A point."""

    dims = 2

    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y
        self.tags: list[str] = []

    def norm1(self) -> int:
        return abs(self.x) + abs(self.y)

    def moved(self, dx: int, dy: int) -> "Point":
        return Point(self.x + dx, self.y + dy)

    def tag_count(self) -> int:
        return len(self.tags)

    def per(self, d: int) -> int:
        return self.x // d

    def __repr__(self) -> str:
        return f"Point({self.x}, {self.y})"

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Point) and self.x == other.x and self.y == other.y

    def __lt__(self, other: "Point") -> bool:
        return self.norm1() < other.norm1()


class Loose:
    def __init__(self, v):
        self.v = v

    def get(self):
        return self.v


def walk(n: int) -> list[Point]:
    out = []
    p = Point(0, 0)
    for i in range(n):
        p = p.moved(i, -i)
        out.append(p)
    return out


def tags_seen(ps: list[Point]) -> int:
    return sum(p.tag_count() for p in ps)


pts = walk(5)
print(sum(p.norm1() for p in pts))
loose = Loose(3)

# Fallback code holding compiled instances.
for p in pts:
    p.tags.append(eval("'seen'"))
    p.x += eval("1")
print(eval("pts"), sorted(pts)[0], pts[0] == Point(1, 0), isinstance(pts[1], Point))
print(type(pts[0]).norm1(pts[0]), type(pts[0]).__name__, Point.dims, pts[2].tags)
q = eval("Point(7, -2)")
print(q.moved(1, 1), q.norm1(), loose.get(), type(loose).get(loose), eval("loose.v"))

# Compiled code sees what fallback code stored.
print(tags_seen(pts), pts[4].x, pts[4].tag_count())

# A method fallback code calls raises through it.
print(eval("pts[1].per(2)"))
eval("pts[0].per(0)")
