import dataclasses
from dataclasses import asdict, astuple, dataclass, field, fields, replace
@dataclass
class Point:
    x: int
    y: int


@dataclass(frozen=True)
class Pixel:
    x: int
    y: int
    tone: str = "red"


@dataclass(eq=False)
class Path:
    start: Point
    hops: list[Point]
    weights: dict[str, int] = field(default_factory=dict)
    label: str = "p"


@dataclass(order=True)
class Version:
    major: int
    minor: int = 0
    tag: str = field(default="", compare=False)
    build: int = field(default=0, init=False)


class Patch(Version):
    pass


def main() -> None:
    p = Point(1, 2)
    # Homogeneous dicts and tuples.
    print(asdict(p), astuple(p), dataclasses.astuple(p))
    print(dataclasses.asdict(Point(-3, 0)))
    # Nested dataclasses, lists and dicts are converted and copied.
    path = Path(Point(0, 0), [Point(1, 1), Point(2, 4)], {"a": 1})
    t = astuple(path)
    print(t)
    t[1].append((9, 9))
    t[2]["b"] = 2
    print(len(path.hops), path.weights, t[1][-1])
    no_hops: list[Point] = []
    print(astuple(Path(Point(5, 5), no_hops, label="q")))
    print(astuple(Pixel(3, 4)), astuple(Pixel(0, 0, "blue")))
    # fields() of an instance and of a class.
    print([f.name for f in fields(path)], len(fields(Point)), fields(Pixel)[2].name)
    for f in dataclasses.fields(Version):
        print("field", f.name)
    # replace() makes a new instance; the original is unchanged.
    q = replace(p, y=9)
    print(q, p, q == Point(1, 9), q is p)
    print(replace(p), replace(p) == p, replace(p) is p)
    print(replace(Pixel(1, 1), tone="blue", x=5))
    v = replace(Version(1, 2, "rc"), minor=3)
    print(v, v > Version(1, 2), v.tag, v.build)
    print(replace(Patch(2), tag="x"))
    moved = dataclasses.replace(path, start=Point(1, 0))
    print(moved.start)
    # A frozen instance can still be replaced, and stays frozen.
    try:
        replace(Pixel(1, 2), x=0).x = 5  # type: ignore[misc]
    except AttributeError as e:
        print("frozen:", e)


main()
