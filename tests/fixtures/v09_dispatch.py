# v0.9: a subclass instance reaching a base-typed compiled value (an `--ext`
# parameter, `Base.method(obj)` from Python, an `object` narrowed by
# `isinstance`) runs the subclass's overrides, as CPython's lookup does
# (`pycc_mir::dispatch`).


class Shape:
    def __init__(self, n: int) -> None:
        self.n = n
        self.log: list[str] = []

    def name(self) -> str:
        return "shape"

    def sides(self) -> int:
        return 0

    def note(self, text: str) -> None:
        self.log.append("shape:" + text)

    def describe(self) -> str:
        self.note("describe")
        return self.name() + "/" + str(self.sides()) + "/" + str(self.n)

    @property
    def label(self) -> str:
        return "a shape"

    def __eq__(self, other: object) -> bool:
        return self is other


class Polygon(Shape):
    def sides(self) -> int:
        return self.n

    @property
    def label(self) -> str:
        return "a polygon"


class Square(Polygon):
    def name(self) -> str:
        return "square"

    def sides(self) -> int:
        return 4

    def note(self, text: str) -> None:
        self.log.append("square:" + text)

    def __eq__(self, other: object) -> bool:
        return True


class Odd(Shape):
    def name(self) -> str:
        return "odd(" + super().name() + ")"


def describe(s: Shape) -> str:
    return s.describe()


def name_of(s: Shape) -> str:
    return s.name()


def label_of(s: Shape) -> str:
    return s.label


def equal(s: Shape, t: Shape) -> bool:
    return s == t


def logged(s: Shape) -> list[str]:
    s.describe()
    return s.log


def via_object(o: object) -> str:
    if isinstance(o, Shape):
        return o.describe()
    return "not a shape"


def run() -> list[str]:
    return [
        describe(Shape(1)),
        Square(3).describe(),
        Polygon(6).describe(),
        Odd(2).describe(),
        name_of(Shape(4)),
    ]


print(run())
