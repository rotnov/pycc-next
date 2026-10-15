# Call-before-def `NameError` reached from a method, a nested function and
# a function called through a value: each raises CPython's catchable
# `NameError` while the callee's `def` has not run.
from typing import Callable


class Box:
    def __init__(self, v: int) -> None:
        self.v = helper(v)

    def get(self) -> int:
        return helper(self.v)


def outer() -> int:
    def inner(k: int) -> int:
        return helper(k)

    return inner(3)


def _apply(fn: Callable[[int], int], v: int) -> int:
    return fn(v)


def via_value(v: int) -> int:
    return helper(v)


try:
    Box(1)
except NameError as e:
    print("init:", e)

try:
    print(outer())
except NameError as e:
    print("closure:", e)

try:
    print(_apply(via_value, 4))
except NameError as e:
    print("value:", e)

f: Callable[[int], int] = via_value
try:
    print(f(5))
except NameError as e:
    print("bound value:", e)


def helper(x: int) -> int:
    return x + 1


print(Box(1).get())
print(outer())
print(_apply(via_value, 4))
print(f(5))
