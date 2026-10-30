from typing import Any, Generic, TypeVar
import typing

# Read by a statement that runs in CPython, so the import runs there and
# `TypeVar` and `Generic` live in the module's namespace.
Key = tuple[Any, int]

T = TypeVar("T")
U = typing.TypeVar("U", bound=int)


class Box(Generic[T]):
    def __init__(self, value: T) -> None:
        self.value = value

    def get(self) -> T:
        return self.value


def first(items: list[T]) -> T:
    return items[0]


def type_var() -> object:
    return T
