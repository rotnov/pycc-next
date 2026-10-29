# v0.10: `min`/`max` with a key returning class instances compare each key
# as soon as it is computed, as CPython does: the key calls and the
# comparisons interleave (`key 0`, `key 1`, `lt`, `key 2`, `lt`, ...), the
# first of equal keys wins, and an exception from a key or a comparison
# stops the loop where CPython's stops.

log: list[str] = []


class Rank:
    def __init__(self, n: int) -> None:
        self.n = n

    def __lt__(self, other: "Rank") -> bool:
        log.append("lt " + str(self.n) + " " + str(other.n))
        if self.n < 0 or other.n < 0:
            raise ValueError("negative rank")
        return self.n < other.n


class Tall:
    """Defines only `__gt__`: `min` falls back to the reflected `__gt__`."""

    def __init__(self, n: int) -> None:
        self.n = n

    def __gt__(self, other: "Tall") -> bool:
        log.append("gt " + str(self.n) + " " + str(other.n))
        return self.n > other.n


def rank(v: int) -> Rank:
    log.append("key " + str(v))
    if v == 99:
        raise KeyError(str(v))
    return Rank(v % 10)


def tall(v: int) -> Tall:
    log.append("tall " + str(v))
    return Tall(v)


def show(result: int) -> None:
    print(result, log)
    log.clear()


show(min([13, 2, 21, 5], key=rank))
show(max([13, 2, 21, 5], key=rank))
show(min([3, 1, 1, 2], key=tall))
show(max([3, 1, 3, 2], key=tall))
show(min([7], key=rank))
try:
    min([4, 99, 1], key=rank)
except KeyError as e:
    print("KeyError", e, log)
    log.clear()
try:
    max([4, -1, 1], key=lambda v: Rank(v))
except ValueError as e:
    print("ValueError", e, log)
    log.clear()
try:
    min([v for v in range(0)], key=rank)
except ValueError as e:
    print("ValueError", e, log)
    log.clear()
print(min(["bb", "a", "ccc"], key=len), max([(1, "x"), (1, "y")], key=lambda p: p[0]))
