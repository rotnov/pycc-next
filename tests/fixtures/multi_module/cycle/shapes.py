"""Imports `geometry`, which imports this module back while it runs."""

BASE = 10
NAME = "shapes"


def double(n: int) -> int:
    return n * 2


print("shapes: before geometry")
import geometry  # noqa: E402

print("shapes: after geometry", geometry.START)
LATE = 3


def total() -> int:
    return geometry.scaled(LATE) + geometry.START


def start() -> int:
    return geometry.START


def describe(n: int) -> str:
    return geometry.label(n) + "/" + NAME
