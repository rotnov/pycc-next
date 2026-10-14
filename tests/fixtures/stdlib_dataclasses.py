from dataclasses import FrozenInstanceError, dataclass, field


@dataclass
class Point:
    x: int
    y: int = 0
    label: str = "p"


@dataclass()
class Bag:
    name: str
    items: list[int] = field(default_factory=list)
    count: int = field(default=3)
    weight: float = field(default=0.5, repr=False, compare=False)


def default_scores() -> dict[str, int]:
    return {"start": 1}


@dataclass
class Scores:
    owner: str
    scores: dict[str, int] = field(default_factory=default_scores)
    total: int = field(init=False, default=0)

    def __post_init__(self) -> None:
        self.total = len(self.scores)


@dataclass(order=True)
class Version:
    major: int
    minor: int
    tag: str = ""


@dataclass(frozen=True)
class Frozen:
    a: int
    b: str = "x"


@dataclass(eq=False)
class Handle:
    n: int


@dataclass
class Point3(Point):
    z: float = 1.5


class Named(Point):
    pass


@dataclass
class Line:
    start: Point
    end: Point

    def length2(self) -> int:
        dx = self.end.x - self.start.x
        dy = self.end.y - self.start.y
        return dx * dx + dy * dy


@dataclass
class Empty:
    pass


def main() -> None:
    p = Point(1)
    print(p)
    print(Point(1, 2, "q"), Point(x=3, label="z"), Point(label="it's", x=0))
    print(p == Point(1), p == Point(2), p != Point(1, 0))
    print(repr(p), str(p), f"{p} {p!r}")
    print(p.x + p.y, p.label)
    p.y = 7
    print(p)
    b1 = Bag("a")
    b2 = Bag("b")
    b1.items.append(7)
    print(b1, b2, b1.count, b1.weight)
    print(Bag("c", [1, 2], 4, 9.5), Bag("a", [7]) == b1, Bag("a", [7], 3, 1.0) == b1)
    s = Scores("me")
    print(s, s.total)
    print(Scores("you", {"a": 1, "b": 2}))
    v1 = Version(1, 2)
    v2 = Version(1, 10, "rc")
    print(v1 < v2, v1 <= v2, v1 > v2, v1 >= v2, v1 == Version(1, 2))
    print(Version(1, 2, "a") < Version(1, 2, "b"), Version(2, 0) > Version(1, 99))
    f = Frozen(1)
    print(f, f.a, f.b, f == Frozen(1), f != Frozen(1, "y"))
    print(hash(f) == hash(Frozen(1)), hash(f) == hash((1, "x")))
    frozen_set = {f, Frozen(1), Frozen(2)}
    print(len(frozen_set))
    try:
        f.a = 5
    except FrozenInstanceError:
        print("frozen")
    try:
        f.b = "y"
    except AttributeError as e:
        print("attr:", e)
    h = Handle(1)
    print(h == Handle(1), h == h, h)
    print(Point3(1, 2, "w"), Point3(4), Point3(4) == Point3(4, 0, "p", 1.5))
    print(Named(5), Named(5) == Named(5))
    line = Line(Point(0, 0), Point(3, 4))
    print(line, line.length2(), line == Line(Point(0), Point(3, 4)))
    print(Empty(), Empty() == Empty())
    f.a = 9


main()
