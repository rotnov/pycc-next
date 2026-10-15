"""`a, b = x, y` where the values are CPython objects (non-strict mode):
compiled code binds the targets from the elements without building a
tuple. Every element is evaluated before any target is bound, as in
CPython; every case prints what CPython prints."""

import collections
from typing import Any


def _split(key: Any) -> str:
    parent, stem = key[:-1], key[-1]
    return f"{parent!r} {stem!r}"


def _swap(a: Any, b: Any) -> str:
    a, b = b, a
    return f"{a!r} {b!r}"


def _mixed(o: Any, n: int) -> str:
    x, y, z = o, n + 1, "s"
    return f"{x!r} {y!r} {z!r}"


def _raising(o: Any) -> str:
    first = "unset"
    second = "unset"
    try:
        first, second = o[0], o[99]
    except IndexError as e:
        return f"IndexError {e} {first} {second}"
    return f"{first!r} {second!r}"


def _loop(items: Any) -> int:
    total = 0
    left, right = items[0], items[-1]
    for _ in range(3):
        left, right = right, left
        total += len(left)
    return total


def main():
    print(_split(("a", "b", "c")))
    print(_split(["x"]))
    print(_split("word"))
    try:
        print(_split(()))
    except IndexError as e:
        print("IndexError", e)
    print(_swap(1, "two"))
    print(_swap(None, collections.OrderedDict(a=1)))
    print(_mixed([1, 2], 41))
    print(_raising((1, 2)))
    print(_raising("ab"))
    print(_loop(["abc", "de"]))


main()
