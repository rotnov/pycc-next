# Issue #10 in a native --non-strict build: `issue_10_api.py`'s definitions
# (without parameter defaults, which keep a class in CPython) compiled, and
# `issue_10_driver.py`'s wrong calls and missing attributes made by
# fallback code: each wrong call is refused compiled, so it runs in CPython
# against the published functions and classes. Generated from those two
# fixtures; keep them in step.


def none() -> int:
    return 0


def one(x: int) -> int:
    return x


def two(x: int, y: int) -> int:
    return x + y


def three(a: int, b: str, c: float) -> str:
    return b * a + str(c)


class Grid:
    def __init__(self, w: int, h: int) -> None:
        self.w = w
        self.h = h

    def area(self) -> int:
        return self.w * self.h

    def scale(self, by: int) -> int:
        return self.area() * by

    @staticmethod
    def unit(n: int) -> int:
        return n

    @classmethod
    def make(cls, n: int) -> int:
        return n * 2


class Box:
    def __init__(self, v: int) -> None:
        self.v = v

    def get(self) -> int:
        return self.v

g = Grid(2, 3)
b = Box(3)
calls = [
    lambda: none(1),
    lambda: none(1, 2),
    lambda: none(k=1),
    lambda: one(),
    lambda: one(1, 2),
    lambda: one(y=1),
    lambda: one(1, x=2),
    lambda: two(),
    lambda: two(1),
    lambda: two(1, 2, 3),
    lambda: two(y=1),
    lambda: two(1, 2, y=3),
    lambda: two(1, 2, 3, z=4),
    lambda: three(),
    lambda: three(1),
    lambda: three(1, c=2.0),
    lambda: Grid(),
    lambda: Grid(1),
    lambda: Grid(1, 2, 3),
    lambda: Grid(1, h=2, w=3),
    lambda: Grid(1, q=2),
    lambda: Box(1, 2),
    lambda: Box(w=1),
    lambda: g.area(1),
    lambda: g.scale(),
    lambda: g.scale(1, 2),
    lambda: g.scale(b=1),
    lambda: Grid.scale(g),
    lambda: Grid.unit(),
    lambda: Grid.unit(1, 2),
    lambda: g.unit(1, 2),
    lambda: Grid.make(),
    lambda: Grid.make(1, 2),
    lambda: b.get(1),
    lambda: getattr(g, "depth"),
    lambda: getattr(Grid, "depth"),
    lambda: getattr(Grid, "aera"),
    lambda: getattr(g, "aera"),
    lambda: getattr(b, "value"),
    lambda: getattr(Box, "value"),
]
for index, call in enumerate(calls):
    try:
        print(index, call())
    except (TypeError, AttributeError) as e:
        print(index, type(e).__name__, e)
getattr(g, "aera")
