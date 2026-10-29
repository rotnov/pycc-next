# Issue #10: published compiled functions, methods and constructors raise
# CPython 3.13's argument-binding TypeErrors word for word, and a missing
# attribute on a compiled class or instance CPython's AttributeError.
# `issue_10_driver.py` imports this module (as source, or built --ext) and
# makes the wrong calls; `issue_10_native.py` makes them from fallback code
# of a native --non-strict build.


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

    def resize(self, w: int, h: int = 1) -> int:
        return w * h

    @staticmethod
    def unit(n: int) -> int:
        return n

    @classmethod
    def make(cls, n: int) -> int:
        return n * 2


class Box:
    def __init__(self, v: int = 3) -> None:
        self.v = v

    def get(self) -> int:
        return self.v
