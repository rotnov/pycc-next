"""Method calls on a class CPython creates (its statement names a metaclass),
whose methods are compiled: each keeps a Python forwarder that calls the
compiled body. Calls from compiled code, from Python, with keywords, with
every count of arguments a call site's cache copies (and one more), and
after whatever can make a cached method stale: an instance attribute of the
same name, a new attribute in the shared keys, a materialized instance
dict, a class attribute rebound.
"""

from typing import Any


def side(x: Any) -> Any:
    return x


class G(metaclass=type):
    def __init__(self, x: Any) -> None:
        self.x = x

    def m0(self) -> Any:
        return ("m0", self.x)

    def m1(self, a: Any) -> Any:
        return ("m1", a)

    def m2(self, a: Any, b: Any = "b") -> Any:
        return ("m2", a, b)

    def m8(self, a: Any, b: Any, c: Any, d: Any, e: Any, f: Any, g: Any, h: Any) -> Any:
        return ("m8", a, b, c, d, e, f, g, h)

    def m9(self, a: Any, b: Any, c: Any, d: Any, e: Any, f: Any, g: Any, h: Any, i: Any) -> Any:
        return ("m9", a, b, c, d, e, f, g, h, i)

    def boom(self, a: Any) -> Any:
        return {}[a]

    @property
    def doubled(self) -> Any:
        return self.x * 2

    def fused(self, n: Any) -> Any:
        out = []
        for i in range(n):
            out.append(self.m0())
            out.append(self.m1(i))
            out.append(self.m2(i))
            out.append(self.m8(i, 1, 2, 3, 4, 5, 6, 7))
            out.append(self.m9(i, 1, 2, 3, 4, 5, 6, 7, 8))
            out.append(self.doubled)
        return out

    def loaded(self, n: Any) -> Any:
        out = []
        for i in range(n):
            out.append(self.m1(side(i)))
            out.append(self.m2(side(i), side("c")))
            out.append(self.m8(side(i), 1, 2, 3, 4, 5, 6, side(7)))
        return out

    def keywords(self, n: Any) -> Any:
        out = []
        for i in range(n):
            out.append(self.m2(i, b=i))
            out.append(self.m2(a=i))
        return out

    def raising(self, k: Any) -> Any:
        try:
            return self.boom(k)
        except KeyError as exc:
            return ("KeyError", exc.args)


def drive(g: Any, n: Any) -> Any:
    out = []
    for i in range(n):
        out.append(g.m1(i))
        out.append(g.m2(i))
    return out


def drive1(g: Any, n: Any) -> Any:
    out = []
    for i in range(n):
        out.append(g.m1(i))
    return out


class S(metaclass=type):
    __slots__ = ("x",)

    def __init__(self, x: Any) -> None:
        self.x = x

    def m1(self, a: Any) -> Any:
        return ("S.m1", self.x, a)


def main() -> None:
    g = G(3)
    print(g.fused(2))
    print(len(g.fused(40)), len(g.loaded(40)), len(drive(g, 40)), len(drive1(S(1), 40)))
    print(g.loaded(2))
    print(g.keywords(2))
    print(g.raising("k"))
    print(drive(g, 2))
    print(drive1(S(4), 2))
    # Python calling the forwarders: positionally, by keyword, wrongly.
    print(g.m2(1), g.m2(1, b=2), g.m2(a=1, b=2), G.m2(g, 5))
    for bad in (lambda: g.m2(), lambda: g.m2(1, 2, 3), lambda: g.m2(1, c=2), lambda: g.m9(1)):
        try:
            bad()
        except TypeError as exc:
            print("TypeError", exc)
    try:
        g.boom("z")
    except KeyError as exc:
        print("KeyError", exc)
    # An instance attribute shadowing a cached method.
    h = G(5)
    print(drive(h, 1))
    h.m1 = lambda a: ("own m1", a)
    print(drive(h, 1), drive(g, 1))
    # A new attribute of another instance grows the shared keys.
    k = G(6)
    print(drive(k, 1))
    g.extra = 1
    print(drive(k, 1), drive(g, 1))
    # A materialized instance dict holding the method's name.
    j = G(7)
    print(drive(j, 1))
    j.__dict__["m2"] = lambda a: ("dict m2", a)
    print(drive(j, 1))
    # The class's methods rebound.
    G.m1 = lambda self, a: ("class m1", a)
    print(drive(G(8), 1), drive1(S(9), 1))
    G.m2 = lambda self, a, b="B": ("class m2", a, b)
    print(drive(G(10), 1))
    print(G(11).fused(1))


if __name__ == "__main__":
    main()
