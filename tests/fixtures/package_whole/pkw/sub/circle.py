from ..base import Shape, BaseError, check, Box, Point
import pkw.base as base_mod

print("init circle", __name__)


class Circle(Shape):
    def __init__(self, r: float) -> None:
        super().__init__("circle")
        self.r = r

    def area(self) -> float:
        return 3.14159 * self.r * self.r


class IntBox(Box[int]):
    def doubled(self) -> int:
        return self.get() * 2


def safe(v: int) -> str:
    try:
        return str(check(v))
    except BaseError as e:
        return "caught " + type(e).__name__ + ": " + str(e)


def safe_value(v: int) -> str:
    try:
        return str(check(v))
    except ValueError as e:
        return "value error " + type(e).__name__ + ": " + str(e)


def safe_plain(v: int) -> str:
    try:
        return str(check(v))
    except ValueError:
        return "plain value error"


def safe_key(v: int) -> str:
    try:
        return str(check(v))
    except KeyError:
        return "key error"


def current_version() -> str:
    return base_mod.version()


def far(p: Point) -> int:
    return p.shifted(10).norm1()
