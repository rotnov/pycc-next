"""Module-level variables of a strict `--ext` module are attributes."""

import json
from enum import Enum

LIMIT = 10
RATE: float = 2.5
NAME = "gauge"
FLAGS = (1, 2, 3)
ENABLED = True
_PRIVATE = 7
counter = 0
pair_a, pair_b = 3, 4
history: list[int] = [1, 2]


class Color(Enum):
    RED = 1


class Box:
    def __init__(self, n: int) -> None:
        self.n = n


DEFAULT_BOX = Box(5)
if LIMIT > 5:
    MODE = "wide"
else:
    MODE = "narrow"
for step in range(3):
    counter += step


def bump(n: int) -> int:
    global counter
    counter += n
    return counter


def rename(name: str) -> None:
    global NAME
    NAME = name


def encoded() -> str:
    return str(json.dumps(LIMIT))
