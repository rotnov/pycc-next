"""Published classes made when their statements run: without arguments,
with `object`, with a base of the module, with a subscripted base (whose
arguments the module evaluates from their source) and with class-body
`None` defaults on instance slots; and one CPython runs (a keyword)."""
import abc
from typing import Generic, TypeVar

T = TypeVar("T")


class Plain:
    def __init__(self, x: int) -> None:
        self.x = x

    def get(self) -> int:
        return self.x


class Shape(object):
    def __init__(self, side: int) -> None:
        self.side = side

    def area(self) -> int:
        return self.side * self.side


class Square(Shape):
    def perimeter(self) -> int:
        return 4 * self.side


class Boxed(Generic[T]):
    def __init__(self, item: int) -> None:
        self.item = item

    def peek(self) -> int:
        return self.item


class Loader(object):
    template = None
    template_name = None

    def __init__(self, name: str) -> None:
        self.template_name = name

    def describe(self) -> str:
        return "loader " + str(self.template_name) + " " + str(self.template)


class Abstract(object, metaclass=abc.ABCMeta):
    def __init__(self, v: int) -> None:
        self.v = v

    def twice(self) -> int:
        return self.v * 2
