"""Method calls whose arguments run code: the method is loaded first (CPython's
LOAD_ATTR), so a missing method raises before the arguments run, and a method
replaced while the arguments run is not the one called."""
import types


def keep(f):
    return f


class Calc:
    @keep
    def unused(self):
        return 0

    def __init__(self):
        self.log = []

    def add(self, a, b):
        return a + b

    def twice(self, v):
        return 2 * v


def side(calc, value):
    Calc.add = lambda self, a, b: "replaced"
    return value


def compute(calc, items):
    total = 0
    for i in items:
        total = calc.add(total, calc.twice(i))
    return total


def reorder(calc):
    return calc.add(1, side(calc, 2))


def missing(calc):
    return calc.nothing(1 // 0)


def many(calc, n):
    return calc.add(n + 1, n * 2), len(str(n))


def via_module(m, x):
    return m.f(x + 1)


def via_dict(d, k):
    return d.get(k + "x", k * 2)


def main():
    c = Calc()
    print(compute(c, range(5)))
    print(many(c, 3))
    original = Calc.add
    print(reorder(c), reorder(c))
    Calc.add = original
    try:
        missing(c)
    except AttributeError as e:
        print("AttributeError", e)
    m = types.ModuleType("mod")
    m.f = lambda v: v * 3
    print(via_module(m, 4), via_module(m, 5))
    print(via_dict({"ax": 1}, "a"), via_dict({}, "b"))
    try:
        print(via_dict(None, "a"))
    except AttributeError as e:
        print("AttributeError", e)


main()
