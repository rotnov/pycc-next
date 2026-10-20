"""The pkw package: every module of it compiled into one extension."""
print("init pkw start", __name__)
from . import base
from .base import Shape, BaseError, VERSION
from .sub import circle

print("init pkw end")


def area_sum(shapes: list) -> float:
    total = 0.0
    for s in shapes:
        total += s.area()
    return total


def describe_all(shapes: list) -> list:
    return [s.describe() for s in shapes]
