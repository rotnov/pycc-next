from dataclasses import dataclass


@dataclass(order=True)
class Point:
    x: int
    y: int = 0
    label: str = "p"


@dataclass(frozen=True)
class Key:
    name: str
    n: int = 1


def make(x: int) -> Point:
    return Point(x, x + 1)


def show(p: Point) -> str:
    return repr(p)
