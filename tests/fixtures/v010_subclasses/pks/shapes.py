"""Compiled subclasses of `base`'s classes: native over `base`'s layout."""

from .base import Plain, Shape, describe, total, weigh


class Square(Shape):
    def __init__(self, name: str, side: int) -> None:
        super().__init__(name, side)
        self.side = side

    def kind(self) -> str:
        return "square"

    def factor(self) -> int:
        return self.side

    def describe(self) -> str:
        return "[" + super().describe() + "]"


class Rect(Square):
    def __init__(self, name: str, w: int, h: int) -> None:
        super().__init__(name, w)
        self.h = h

    def area(self) -> int:
        return self.side * self.h

    def kind(self) -> str:
        return "rect/" + super().kind()

    def describe(self) -> str:
        return "R" + super().describe()


class Doubled(Plain):
    def value(self) -> int:
        return 2 * self.n


def run() -> str:
    sq = Square("sq", 3)
    r = Rect("r", 2, 5)
    out = [sq.describe(), r.describe(), describe(sq), describe(r), describe(Shape("s", 4))]
    out.append(str(weigh(sq)) + " " + str(weigh(r)) + " " + str(sq.weigh(2)))
    out.append(str(total(sq, r) + total(r, Shape("t", 1))))
    r.grow(1)
    out.append(r.describe() + " " + str(r.size) + " " + str(r.side))
    d = Doubled(4)
    out.append(str(d.value()) + " " + str(d.n))
    return " | ".join(out)


def kinds(s: Shape) -> str:
    return s.kind() + "/" + str(s.area())
