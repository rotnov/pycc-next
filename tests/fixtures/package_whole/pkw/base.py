from abc import ABC, abstractmethod
from typing import Generic, Protocol, TypeVar

print("init base", __name__)
VERSION = "1.0"
T = TypeVar("T")


class BaseError(ValueError):
    pass


class DeepError(BaseError):
    pass


class Shape(ABC):
    def __init__(self, name: str) -> None:
        self.name = name

    @abstractmethod
    def area(self) -> float: ...

    def describe(self) -> str:
        return f"{self.name} with area {self.area():.2f}"


class Tag:
    def __init__(self, name: str) -> None:
        self.name = name

    def show(self) -> str:
        return "<" + self.name + ">"


class Box(Generic[T]):
    def __init__(self, item: T) -> None:
        self.item = item

    def get(self) -> T:
        return self.item


class Named(Protocol):
    name: str


class Point:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def norm1(self) -> int:
        return abs(self.x) + abs(self.y)

    def shifted(self, d: int) -> "Point":
        return Point(self.x + d, self.y + d)


def check(v: int) -> int:
    if v < 0:
        raise BaseError("negative: " + str(v))
    if v > 100:
        raise DeepError("too big: " + str(v))
    return v * 2


def version() -> str:
    return VERSION


def label(n: int) -> str:
    return "item-" + str(n)
