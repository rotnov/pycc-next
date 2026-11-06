"""A module whose functions a non-strict `--ext` build selects (tests/select_functions.rs)."""
import functools

SCALE = 3


def hot(d, k=1, *rest, flag=False, **kw):
    try:
        return d[k] * SCALE
    except KeyError:
        return None


class Base:
    def greet(self):
        return "base"


class Box(Base):
    def __init__(self, items):
        self.items = items

    def total(self):
        return sum(hot(self.items, k) or 0 for k in range(3))

    def greet(self):
        return "box+" + super().greet()

    @property
    def size(self):
        return len(self.items)

    @staticmethod
    def twice(x):
        return x * 2


def deco(f):
    @functools.wraps(f)
    def wrapper(*a, **k):
        return f(*a, **k)
    return wrapper


@deco
def wrapped(x):
    return x + 1


def boom(x):
    return 1 / x


def main():
    import sys, traceback
    print(__file__.rsplit("/", 1)[-1], __doc__.split()[0])
    print(hot({1: 2}), hot({2: 5}, 2), hot({}, 3), hot({1: 2}, flag=True))
    b = Box({0: 1, 1: 2})
    print(b.total(), b.greet(), b.size, Box.twice(4), wrapped(1), wrapped.__name__)
    print(sorted(n for n in globals() if n.startswith("__pycc")))
    try:
        boom(0)
    except ZeroDivisionError:
        traceback.print_exc(file=sys.stdout)
