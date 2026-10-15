"""Imports `shapes` while `shapes` is still running its body: only the
names `shapes` bound before `import geometry` exist yet."""

import shapes
from shapes import BASE, double

print("geometry: shapes is partially initialized")
START = double(BASE) + shapes.BASE
print("geometry:", START, shapes.NAME)


def scaled(n: int) -> int:
    # Runs later, when `shapes` has finished: `LATE` exists by then.
    return shapes.double(n) + shapes.LATE


def label(n: int) -> str:
    return shapes.NAME + ":" + str(scaled(n))
