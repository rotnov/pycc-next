"""A third module: a subclass of a subclass another module compiled."""

from .shapes import Square


class Cube(Square):
    def kind(self) -> str:
        return "cube"

    def area(self) -> int:
        return 6 * self.side * self.side
