from enum import IntEnum


class Level(IntEnum):
    LOW = 1
    MID = 2
    HIGH = 3


def bump(n: int) -> int:
    return Level.HIGH + n


def at_least_mid(n: int) -> bool:
    return n >= Level.MID


def label(n: int) -> str:
    return ["a", "b", "c", "d", "e"][Level.LOW + n]


def level_text(n: int) -> str:
    return str(Level(n))
