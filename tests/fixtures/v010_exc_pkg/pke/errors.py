"""Exception classes deriving from `common`'s."""
from .common import Owned, PkgError


class KeyMissing(PkgError):
    """Over a base with no layout: a layout of its own."""

    def __init__(self, key: str, details: str) -> None:
        self.key = key
        self.details = details

    def __str__(self) -> str:
        return "Key " + repr(self.key) + " not found: " + self.details


class Sub(Owned):
    """Over another module's layout class."""

    def __init__(self, code: int, where: str, extra: str) -> None:
        super().__init__(code, where)
        self.extra = extra

    def describe(self) -> str:
        return super().describe() + "+" + self.extra


class Clash(Exception):
    """Named as a class of `common` is: each has a layout of its own."""

    def __init__(self, b: str, c: str) -> None:
        self.b = b
        self.c = c


def lookup(d: dict[str, int], key: str) -> int:
    if key not in d:
        raise KeyMissing(key, "in lookup")
    return d[key]


def raise_sub(n: int) -> None:
    raise Sub(n, "errors", "x")


def catch(n: int) -> str:
    try:
        raise_sub(n)
    except Owned as e:
        return "owned " + e.describe()
    return "none"
