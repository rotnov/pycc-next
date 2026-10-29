# v0.10: a `StopIteration` a `min`/`max` key raises (here `next()` of an
# exhausted generator) propagates as it is, as from CPython's builtins: the
# generator pycc pulls the `(key, element)` pairs from does not turn it
# into `RuntimeError` (PEP 479 applies to generators the program writes,
# and CPython runs the key in none).
from typing import Iterator

log: list[str] = []


class Rank:
    def __init__(self, n: int) -> None:
        self.n = n

    def __lt__(self, other: "Rank") -> bool:
        log.append("lt " + str(self.n) + " " + str(other.n))
        return self.n < other.n


def two() -> Iterator[int]:
    yield 5
    yield 4


def smallest(xs: list[int]) -> str:
    ranks = two()

    def key(v: int) -> Rank:
        log.append("key " + str(v))
        return Rank(next(ranks))

    try:
        return str(min(xs, key=key))
    except StopIteration:
        return "StopIteration"
    except RuntimeError as e:
        return "RuntimeError " + str(e)


def longest(xs: list[str]) -> str:
    sizes = two()

    def size(v: str) -> int:
        log.append("size " + v)
        return next(sizes)

    try:
        return max(xs, key=size)
    except StopIteration:
        return "StopIteration"
    except RuntimeError as e:
        return "RuntimeError " + str(e)


print(smallest([1, 2, 3]), log)
log.clear()
print(smallest([7, 8]), log)
log.clear()
print(longest(["a", "bb", "c"]), log)
log.clear()
print(longest(["a", "ccc"]), log)
