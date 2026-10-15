"""A module function called through an import alias shares its name with a
method whose signature is bound at its call sites (keyword-only parameter,
default): the import's call is a foreign call, never bound to the method."""

import math as m
from math import floor


class Box:
    def __init__(self, x: float) -> None:
        self.x = x

    def sqrt(self, *, scale: float = 1.0) -> float:
        return self.x * scale

    def floor(self, digits: int = 0) -> float:
        return round(self.x, digits)


def main() -> None:
    b = Box(4.25)
    print(m.sqrt(16.0), b.sqrt(), b.sqrt(scale=2.0))
    print(floor(2.5), b.floor(), b.floor(1))
    try:
        print(m.sqrt(-1.0))
    except ValueError as e:
        print("ValueError", e)


main()
