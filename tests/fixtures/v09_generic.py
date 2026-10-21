# v0.9 accept 1: classes with `Generic[...]` bases -- plain and constrained
# type variables -- stay native, and the published class is made with its
# real bases, so CPython code sees `Generic`'s machinery (`C[int]`,
# `__parameters__`, `__orig_bases__`); compiled subclasses of compiled
# classes are CPython subclasses too.
from typing import Generic, TypeVar

T = TypeVar("T")
N = TypeVar("N", int, float)
K = TypeVar("K", bound=str)


class Box(Generic[T]):
    def __init__(self, item):
        self.item = item

    def get(self):
        return self.item

    def map(self, f):
        return Box(f(self.item))


class Acc(Generic[N]):
    def __init__(self, start):
        self.total = start

    def add(self, x):
        self.total = self.total + x
        return self


class Pair(Generic[K, T]):
    def __init__(self, key, value):
        self.key = key
        self.value = value

    def swap(self):
        return Pair(self.value, self.key)


class Base:
    def __init__(self, n):
        self.n = n

    def describe(self):
        return "base " + str(self.n)

    def twice(self):
        return self.n * 2


class Derived(Base):
    def describe(self):
        return "derived " + str(self.n)


def run():
    b = Box[int](5)
    a = Acc(1.5).add(2.0).add(3.5)
    p = Pair("k", 1).swap()
    d = Derived(4)
    upper = Box("s").map(str.upper)
    return [b.get(), upper.get(), a.total, p.key, p.value, d.describe(), d.twice()]


print(run())
