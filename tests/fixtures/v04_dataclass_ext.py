from dataclasses import asdict, astuple, dataclass, fields, replace


@dataclass
class Point:
    x: int
    y: int


def as_dict(x: int, y: int) -> str:
    return str(asdict(Point(x, y)))


def as_tuple(x: int, y: int) -> tuple[int, int]:
    return astuple(Point(x, y))


def names() -> str:
    return ",".join([f.name for f in fields(Point)])


def moved(x: int, y: int, dy: int) -> str:
    p = Point(x, y)
    return repr(replace(p, y=p.y + dy))
