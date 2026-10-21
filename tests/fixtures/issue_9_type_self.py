# Issue #9: in a --non-strict native build with no fallback code, CPython
# code meeting an instance of a compiled class (through `type(obj)`,
# `getattr`, `operator.methodcaller`) finds the class's methods, as it finds
# a Python class's.
import operator


class Counter:
    def __init__(self, start: int) -> None:
        self.n = start

    def bump(self, by: int) -> int:
        self.n += by
        return self.n

    def again(self) -> int:
        return type(self).bump(self, 10)

    def via_getattr(self, name: str) -> int:
        return getattr(type(self), name)(self, 1)

    def fail(self) -> int:
        raise ValueError("boom " + str(self.n))

    @staticmethod
    def zero() -> int:
        return 0

    @classmethod
    def label(cls) -> str:
        return "counter"

    def __repr__(self) -> str:
        return "Counter(" + str(self.n) + ")"


c = Counter(3)
m = getattr(type(c), "bump")
print(m(c, 2), c.again(), c.via_getattr("bump"))
print(operator.methodcaller("bump", 5)(c), type(c).__name__, type(c).zero(), type(c).label())
print(c)
print(getattr(type(c), "missing", "no such method"))
getattr(type(c), "fail")(c)
