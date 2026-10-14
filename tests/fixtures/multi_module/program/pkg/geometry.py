print("pkg.geometry: body")
import shared
from .util import triple
from . import util


class Point:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def norm2(self) -> int:
        return self.x * self.x + self.y * self.y + triple(1) - util.triple(1)

    def shifted(self, d: int) -> "Point":
        return Point(self.x + d, self.y + d)


def origin_distance(p: Point) -> int:
    return p.norm2()


def shared_calls() -> int:
    return shared.calls()
