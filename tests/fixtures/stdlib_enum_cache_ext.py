from enum import Enum, IntEnum
from functools import lru_cache


class Color(Enum):
    RED = 1
    GREEN = 2


class Level(IntEnum):
    LOW = 1
    HIGH = 5


def pick(n: int) -> str:
    c = Color(n)
    return str(c) + " " + repr(c) + " " + c.name


def same(a: int, b: int) -> bool:
    return Color(a) is Color(b)


def level_text(n: int) -> str:
    return str(Level(n)) + " " + repr(Level(n))


@lru_cache(maxsize=None)
def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)
