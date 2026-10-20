"""Calls into other modules of the package: native while the names bind the functions
those modules published, through the objects once Python code rebinds them."""

from .util import scaled, shout, ratio, is_short, note, ident, fails
from .util import shout as _shout
from .base import label


def use(n: int, s: str) -> str:
    note(s)
    return (label(scaled(n)) + shout(s) + _shout(s) + str(ratio(1.0, 4.0)) + str(is_short(s))
            + str(ident(n)))


def loop(n: int) -> int:
    t = 0
    for i in range(n):
        t += scaled(i)
    return t


def keyword(s: str) -> str:
    return shout(s=s)


def failing(s: str) -> str:
    return fails(s)


def div(a: float, b: float) -> float:
    return ratio(a, b)


def private(s: str) -> str:
    return _shout(s)


def token(s: str, extra):
    # A native `str` result here would make the inner display a native dict
    # among objects: the call stays a call of the object.
    return {"type": "link", "attrs": {"url": shout(s)}, "extra": extra}


def early() -> str:
    return shout("early")


print("init calls", early())
