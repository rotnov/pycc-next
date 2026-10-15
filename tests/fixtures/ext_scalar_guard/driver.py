"""Calls `m` (the directory in argv[1]) as Python code would; prints what
it gets."""

import sys
from decimal import Decimal

sys.path.insert(0, sys.argv[1])
import m  # noqa: E402


def attempt(f, *args, **kwargs):
    try:
        print(repr(f(*args, **kwargs)))
    except Exception as e:  # noqa: BLE001
        print(type(e).__name__, e)


attempt(m.clean, "a\r\nb")
attempt(m.clean, "a\r\nb", sep="|")
attempt(m.clean, b"x")
attempt(m.clean, None)
attempt(m.clean, "a\r\nb", sep=1)
attempt(m.clean)
attempt(m.scale, 1.5)
attempt(m.scale, 2, 3, label="y")
attempt(m.scale, True)
attempt(m.scale, Decimal("1.5"))
attempt(m.scale, "ab", "c")
attempt(m.scale, "ab")
attempt(m.count, "a", "b")
attempt(m.count, "a", flag=1)
attempt(m.count, "a", 2)
attempt(m.tag, "a", x=1, y=2)
attempt(m.tag, "a", x=1.5)
attempt(m.tag, "a", x="s")
for value in ({}, [1], (1,), True, 2, 2.5, "s", m.Box(1), None, b"x"):
    attempt(m.kind, value)
attempt(m.pick, [5, 6], 1)
attempt(m.pick, "xyz", 1.0)
attempt(m.pick, "xyz", i=True)
