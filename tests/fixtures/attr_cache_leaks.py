"""Reference counts across cached attribute reads and method calls: every hit
returns a new reference the caller releases, so nothing a read touches may gain
or lose references."""
import sys
import types


def keep(f):
    return f


class Plain:
    @keep
    def unused(self):
        return 0

    def __init__(self, v):
        self.v = v

    def get(self):
        return self.v

    @property
    def p(self):
        return self.v


class Slotted:
    __slots__ = ("v",)

    @keep
    def unused(self):
        return 0

    def __init__(self, v):
        self.v = v


def read_v(o):
    return o.v


def read_p(o):
    return o.p


def call_get(o):
    return o.get()


def bound_get(o):
    return o.get


def call_mod(m):
    return m.f(1)


def loop(fn, arg, n):
    for _ in range(n):
        fn(arg)


def main():
    value = object()
    plain = Plain(value)
    slotted = Slotted(value)
    module = types.ModuleType("leaky")
    module.v = value
    module.f = lambda x: value
    cases = [
        ("value", read_v, plain),
        ("slot", read_v, slotted),
        ("module", read_v, module),
        ("property", read_p, plain),
        ("method", call_get, plain),
        ("bound", bound_get, plain),
        ("module call", call_mod, module),
    ]
    getter = Plain.__dict__["p"].fget
    method = Plain.__dict__["get"]
    for name, fn, arg in cases:
        loop(fn, arg, 100)
        before = (sys.getrefcount(value), sys.getrefcount(arg), sys.getrefcount(getter),
                  sys.getrefcount(method))
        loop(fn, arg, 10000)
        after = (sys.getrefcount(value), sys.getrefcount(arg), sys.getrefcount(getter),
                 sys.getrefcount(method))
        print(name, "balanced" if before == after else f"{before} -> {after}")


main()
