# Issue #1: a function value is the function its name held when the value
# was made, also for a function the module defines more than once. Typed:
# built strict and --non-strict.
from typing import Callable


def apply(f: Callable[[int], int], v: int) -> int:
    return f(v)


def shape(x: int) -> int:
    return x + 1


first = shape
values: list[Callable[[int], int]] = [shape]


def shape(x: int) -> int:
    return x * 2


values.append(shape)
second = shape


def shape(x: int) -> int:
    return x * 3


print(first(10), second(10), shape(10))
print(apply(values[0], 4), apply(values[1], 4))
print(apply(first, 5), apply(shape, 5))


def make_adder(n: int) -> Callable[[int], int]:
    def add(x: int) -> int:
        return x + n
    return add


print(make_adder(2)(3))
