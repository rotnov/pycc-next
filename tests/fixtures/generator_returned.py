"""A function that returns a generator without yielding itself is an
ordinary function: its `return` hands the generator back, it does not end
a generator of its own."""
from typing import Iterator


def lengths(rows: list[list[int]]) -> Iterator[int]:
    for row in rows:
        yield len(row)


def through(rows: list[list[int]]) -> Iterator[int]:
    return lengths(rows)


def through_local(rows: list[list[int]]) -> Iterator[int]:
    g = lengths(rows)
    return g


def choose(rows: list[list[int]], twice: bool) -> Iterator[int]:
    if twice:
        return lengths(rows + rows)
    return lengths(rows)


xs = [[1], [2, 3], []]
print(list(through(xs)), list(through_local(xs)), list(choose(xs, True)))
g = through([[4, 5, 6]])
print(next(g))
try:
    next(g)
except StopIteration:
    print("done")
for n in choose(xs, False):
    print(n)
