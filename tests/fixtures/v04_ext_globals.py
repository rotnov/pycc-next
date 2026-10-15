"""Module-level variables of a strict `--ext` module are attributes.

`json`, a CPython module, and `DEFAULT_BOX`/`SHADE`, which cross the boundary
as CPython objects, are not published: see `docs/DEVIATIONS.md`.
"""

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


def level() -> int:
    return LIMIT // 3


class Box:
    def __init__(self, n: int) -> None:
        self.n = n


DEFAULT_BOX = Box(5)
SHADE = Color.RED
SIZE = len(NAME) * 2
LEVEL = level()
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
