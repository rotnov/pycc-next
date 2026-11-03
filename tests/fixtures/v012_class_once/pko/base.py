from typing import Generic, TypeVar

T = TypeVar("T")


class Plain:
    """A plain class: made in one step."""

    def __init__(self, n: int) -> None:
        self.n = n
        self.label = "p"

    def value(self) -> int:
        return self.n * 2

    def show(self) -> str:
        return "Plain(" + str(self.n) + ")"


class Explicit(object):
    kind = "explicit"
    LIMIT = 10
    template = None

    def __init__(self, s: str) -> None:
        self.s = s

    def text(self) -> str:
        return self.s + "!"


def mark(cls):
    return cls


@mark
@mark
class Counter:
    def __init__(self) -> None:
        self.count = 0

    def bump(self) -> int:
        self.count += 1
        return self.count


class Child(Plain):
    """  Derived in its module.

      Indented further, then a tab:
	after it.
    """

    def value(self) -> int:
        return self.n * 3


class Holder(Generic[T]):
    """Made from its template: `Generic[T]` is evaluated from its source."""

    def __init__(self, item: T) -> None:
        self.item = item

    def get(self) -> T:
        return self.item


class Kept(Holder):
    """Made from its template: its base is not a class made in one step."""

    def again(self) -> str:
        return "kept " + str(self.get())


def total(p: Plain) -> int:
    return p.value()


def counted(c: Counter) -> int:
    return c.bump() + c.bump()
