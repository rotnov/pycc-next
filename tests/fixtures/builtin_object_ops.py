"""Object operations the shim answers itself for exact built-in types:
an `int` index into a `tuple` or `list` (negative, out of range, the empty
ones, a subclass), `isinstance` of a built-in class (`bool` is an `int`, a
subclass instance, every plain built-in type), and the standard-library
functions compiled code calls, many of them at many sites.
"""

import bisect
import math
from typing import Any


class MyList(list):
    def __getitem__(self, i: Any) -> Any:
        return ("MyList", i)


class MyInt(int):
    pass


def index(seq: Any, i: int) -> Any:
    try:
        return seq[i]
    except IndexError as exc:
        return ("IndexError", str(exc))


def kinds(v: Any) -> Any:
    return (isinstance(v, int), isinstance(v, str), isinstance(v, float), isinstance(v, bool),
            isinstance(v, list), isinstance(v, dict), isinstance(v, tuple))


def library(v: Any) -> Any:
    a = tuple(v)
    b = sorted(v)
    c = list(v)
    d = sum(v)
    e = min(v)
    f = max(v)
    g = len(v)
    h = math.floor(d / 2)
    k = bisect.bisect_left(b, 2)
    return (a, b, c, d, e, f, g, h, k)


def main() -> None:
    for seq in [(1, 2, 3), [1, 2, 3], (), [], MyList([5, 6])]:
        print([index(seq, i) for i in [0, 1, 2, 3, -1, -3, -4]])
    for v in [0, 1, True, False, MyInt(3), "s", 1.5, [1], {"a": 1}, (1,), {1}, frozenset(), None, b"x"]:
        print(type(v).__name__, kinds(v))
    for v in [[3, 1, 2], (5, 4), [], [2.5, 1.0]]:
        try:
            print(library(v))
        except ValueError as exc:
            print("ValueError", exc)


main()
