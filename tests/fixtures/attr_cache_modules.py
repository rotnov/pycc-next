"""Attribute-cache semantics on modules: rebinding, deletion, __getattr__, calls."""
import math
import types


def read_x(m):
    return m.x


def read_pi(m):
    return m.pi


def call_f(m):
    return m.f(2)


def main():
    out = []
    m = types.ModuleType("dyn")
    m.x = 1
    m.f = lambda v: v + 1
    n = types.ModuleType("dyn2")
    n.y = 0
    n.x = "n.x"
    n.f = lambda v: v * 10
    for _ in range(3):
        out.append((read_x(m), read_x(n), call_f(m), call_f(n), read_pi(math)))
    m.x = 2
    out.append(read_x(m))
    del m.x
    try:
        read_x(m)
    except AttributeError as e:
        out.append(("err", str(e)))
    m.__getattr__ = lambda name: "ga:" + name
    out.append(read_x(m))
    m.x = 3
    out.append(read_x(m))
    for i in range(50):
        setattr(m, "pad%d" % i, i)
    out.append((read_x(m), call_f(m)))
    m.f = lambda v: "new"
    out.append(call_f(m))
    del m.f
    try:
        call_f(m)
    except TypeError as e:
        out.append(("err", type(e).__name__))
    for line in out:
        print(line)


main()
