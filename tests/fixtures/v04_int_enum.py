from enum import Enum, IntEnum


class Level(IntEnum):
    LOW = 1
    MID = 2
    HIGH = 3


class Code(IntEnum):
    OK = 0
    WARN = 2


class Color(Enum):
    RED = 1


def scaled(x: int) -> int:
    return x * 10


def main() -> None:
    # Arithmetic, bitwise and unary operators read the value.
    print(Level.HIGH + 1, 1 + Level.HIGH, Level.HIGH - Level.LOW)
    print(Level.MID * 3, Level.HIGH // 2, Level.HIGH % 2, Level.HIGH / 4, Level.MID ** 3)
    print(Level.HIGH & 1, Level.HIGH | 4, Level.MID << 2, Level.HIGH ^ Level.LOW)
    print(-Level.MID, +Level.LOW, ~Level.HIGH, Level.MID * 0.5)
    total = 0
    for level in [Level.LOW, Level.MID, Level.HIGH]:
        total += level
    print(total)
    # Comparisons with numbers, and between members of different classes.
    print(Level.MID == 2, Level.MID != 2, 2 == Level.MID, Level.LOW == 1.0)
    print(Level.LOW < 2, Level.HIGH > 1, 3 >= Level.HIGH, Level.LOW <= 0)
    print(Code.WARN == Level.MID, Code.OK < Level.LOW, 0 <= Code.OK < Level.HIGH)
    # Members of one class still compare as members.
    print(Level.LOW < Level.HIGH, Level.MID == Level.MID, Level.LOW is Level.LOW)
    # Indexing and membership.
    xs = [10, 20, 30, 40]
    print(xs[Level.LOW], xs[-Level.LOW], "abcd"[Level.HIGH])
    seen = {5, 6}
    print(Level.MID in [1, 2], Level.HIGH in seen, Level.LOW not in [1])
    # Conversions and builtins reading an int.
    print(int(Level.HIGH), float(Level.MID), abs(Level.LOW), divmod(Level.HIGH, 2))
    print(list(range(Level.HIGH)), [i for i in range(Level.LOW, Level.HIGH)])
    for i in range(Level.MID):
        print("tick", i)
    # An `int` parameter or name.
    print(scaled(Level.MID))
    n: int = Level.HIGH
    print(n, n * n)
    # The member itself prints as its value (Python 3.11+), repr as a member.
    print(Level.MID, str(Level.MID), repr(Level.MID), f"{Level.HIGH}")
    print(Level.MID.name, Level.MID.value, Level(3) is Level.HIGH)
    print(hash(Level.MID) == hash(2), Color.RED.value + Level.LOW)
    print({Level.HIGH: "h"}[Level.HIGH])
    # An index out of range raises CPython's own error.
    print(xs[Level.HIGH + 4])


main()
