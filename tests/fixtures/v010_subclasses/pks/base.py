"""Base classes other modules of the package subclass."""


class Shape:
    def __init__(self, name: str, size: int) -> None:
        self.name = name
        self.size = size

    def kind(self) -> str:
        return "shape"

    def area(self) -> int:
        return self.size * self.size

    def factor(self) -> int:
        return 1

    def weigh(self, by: int) -> int:
        return by * self.factor()

    def describe(self) -> str:
        return self.kind() + ":" + self.name + "=" + str(self.area())

    def grow(self, by: int) -> None:
        self.size += by


class Plain:
    def __init__(self, n: int) -> None:
        self.n = n

    def value(self) -> int:
        return self.n


def describe(s: Shape) -> str:
    return s.describe()


def weigh(s: Shape) -> int:
    return s.weigh(3) + s.weigh(-1)


def total(a: Shape, b: Shape) -> int:
    return a.area() + b.area()


def make(name: str) -> Shape:
    return Shape(name, 2)
