"""The package's base exception classes."""


class PkgError(Exception):
    """Base class of the package's errors."""


class Owned(Exception):
    """An exception class with attributes of its own."""

    def __init__(self, code: int, where: str) -> None:
        super().__init__(f"code {code}")
        self.code = code
        self.where = where

    def describe(self) -> str:
        return self.where + ":" + str(self.code)


def code_of(e: Owned) -> int:
    return e.code


class Rated(Exception):
    """`errors.Overrated` makes `rate` a property."""

    def __init__(self, rate: int) -> None:
        self.rate = rate

    def show(self) -> str:
        return "rate " + str(self.rate)


class Lefty(Exception):
    """`errors.Duo` derives from it and another base."""

    def __init__(self, left: int) -> None:
        self.left = left

    def show(self) -> str:
        return "left " + str(self.left)


class Pinned(Exception):
    """`errors.PinAlias` binds it to another name."""

    def __init__(self, pin: int) -> None:
        self.pin = pin


class Clash(Exception):
    """Named as a class of `errors` is."""

    def __init__(self, a: int) -> None:
        self.a = a
