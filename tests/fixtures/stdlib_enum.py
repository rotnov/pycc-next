from enum import Enum, IntEnum, StrEnum, auto


class Color(Enum):
    RED = 1
    GREEN = 2
    BLUE = 3


class Kind(Enum):
    AXIAL = "axial"
    RADIAL = "it's"


class Priority(IntEnum):
    LOW = 1
    MEDIUM = auto()
    HIGH = 10


class Mode(StrEnum):
    READ = auto()
    WRITE = "w"


def describe(c: Color) -> str:
    match c:
        case Color.RED:
            return "warm"
        case Color.GREEN | Color.BLUE:
            return "cool"


def main() -> None:
    c = Color.RED
    print(c)
    print(repr(c), str(c), f"{c} {c!r}")
    print(c.name, c.value, Color.BLUE.value)
    print(c == Color.RED, c == Color.GREEN, c != Color.BLUE)
    print(c is Color.RED, c is Color.GREEN, c is not Color.GREEN)
    for m in Color:
        print(m, m.name, m.value)
    print(Color(2), Color(3) is Color.BLUE)
    print(Kind.AXIAL, repr(Kind.RADIAL), Kind("axial"))
    print(Priority.HIGH, repr(Priority.MEDIUM), f"{Priority.LOW}", str(Priority.HIGH))
    print(Priority.LOW < Priority.HIGH, Priority.HIGH <= Priority.MEDIUM, Priority(10))
    print(Mode.READ, repr(Mode.READ), Mode.WRITE.value)
    print(describe(Color.RED), describe(Color.BLUE))
    seen = {Color.RED, Color.GREEN, Color.RED}
    print(len(seen), hash(Priority.HIGH), hash(Mode.WRITE) == hash("w"), hash(Color.RED) == hash("RED"))
    names = {Color.RED: "red", Color.GREEN: "green"}
    names[Color.BLUE] = "blue"
    print(names[Color.GREEN], len(names), Color.BLUE in names)
    counts: dict[Priority, int] = {}
    for p in [Priority.HIGH, Priority.LOW, Priority.HIGH]:
        counts[p] = counts.get(p, 0) + 1
    for p, n in counts.items():
        print(p.name, n)
    print([k.value for k in names])
    print(Color(4))


main()
