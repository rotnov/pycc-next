"""The package's base exception classes."""


class PkgError(Exception):
    """Base class of the package's errors."""


class Owned(Exception):
    """An exception class with a native layout of its own."""

    def __init__(self, code: int, where: str) -> None:
        super().__init__(f"code {code}")
        self.code = code
        self.where = where

    def describe(self) -> str:
        return self.where + ":" + str(self.code)


def code_of(e: Owned) -> int:
    return e.code


class Rated(Exception):
    """Would have a layout, but `errors.Overrated` shadows `rate`."""

    def __init__(self, rate: int) -> None:
        self.rate = rate

    def show(self) -> str:
        return "rate " + str(self.rate)


class Clash(Exception):
    """Named as a class of `errors` is: each has a layout of its own."""

    def __init__(self, a: int) -> None:
        self.a = a
